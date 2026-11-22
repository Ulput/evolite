//! FEAT-0048 step 2: the mixed-coin asset lock, handed to platform-wallet. The transaction and
//! its check are `crate::mixed_lock`; here the credit key, the signature, the registration and
//! the broadcast (`docs/tasks/features/FEAT-0048/step2-design.md`).
//!
//! The order is the architect's (М-2): the lock is built and signed, registered as tracked
//! (`recover_asset_lock_blocking`) BEFORE it is broadcast, then broadcast; the caller then hands
//! it on with `AssetLockFunding::FromExistingAssetLock`, and platform-wallet waits for its
//! InstantSend or ChainLock proof and resumes it after a cut. A broadcast that definitely did not
//! go is untracked again; one whose outcome is unclear stays (BUG-243).

use super::*;
use key_wallet::Utxo;
use key_wallet::account::AccountType;
use key_wallet::dashcore::Address;
use key_wallet::dashcore::OutPoint;
use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
use key_wallet::wallet::managed_wallet_info::asset_lock_builder::AssetLockFundingType;
use key_wallet::wallet::managed_wallet_info::transaction_building::AccountTypePreference;
use platform_wallet::changeset::{
    AccountAddressPoolEntry, AccountRegistrationEntry, AssetLockChangeSet, PlatformWalletChangeSet,
    PlatformWalletPersistence,
};

/// What a mixed-coin lock pays for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixedLockFor {
    /// A new identity in this DIP-9 slot (path M).
    Identity { slot: u32 },
    /// A top-up of the identity in this slot (UC-7).
    TopUp { slot: u32 },
    /// Shielding into the pool (UC-8).
    Pool,
}

impl MixedLockFor {
    fn funding_type(self) -> AssetLockFundingType {
        match self {
            Self::Identity { .. } => AssetLockFundingType::IdentityRegistration,
            Self::TopUp { .. } => AssetLockFundingType::IdentityTopUp,
            Self::Pool => AssetLockFundingType::AssetLockShieldedAddressTopUp,
        }
    }

    fn identity_index(self) -> u32 {
        match self {
            Self::Identity { slot } | Self::TopUp { slot } => slot,
            Self::Pool => 0,
        }
    }
}

/// A mixed-coin lock that went out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MixedLock {
    /// The credit output, as platform-wallet tracks it (txid, 0).
    pub out_point: OutPoint,
    /// What the credit output holds, in duffs: the coins less the Core fee.
    pub credit: u64,
    /// The Core fee.
    pub fee: u64,
    /// The broadcast's outcome was proved (`false`: not known — never sent again, BUG-243).
    pub relayed: bool,
}

/// The account (CoinJoin 0) whose coins pay a mixed lock; also the `account_index` a tracked lock
/// funded from it carries.
const COINJOIN_ACCOUNT: u32 = 0;

impl Chain {
    /// FEAT-0048 step 2: locks exactly `coins` (fully mixed denominations, chosen by the caller)
    /// for `purpose`, registers the lock as tracked and broadcasts it. `Err`: nothing went to the
    /// network, the coins are free and the lock untracked. Runs only inside the job that holds
    /// the identity's or the pool's busy flag: no «Continue» of a lock can run meanwhile, so no
    /// resume of platform-wallet's holds this outpoint while it may be untracked here (the
    /// architect's condition 2).
    pub async fn mixed_lock(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        coins: &[Utxo],
        purpose: MixedLockFor,
    ) -> Result<MixedLock, String> {
        // BUG-273: no new asset lock while this wallet's funding keys are unchecked.
        self.require_funding_keys(wallet).await?;
        let credit_address = self.take_credit_address(wallet, keys, purpose).await?;
        let script = credit_address.script_pubkey();
        // r1: the same lock built unsigned and checked first (its fee under the ceiling):
        // nothing is signed or reserved for a lock that would be refused.
        let (dry, _, _) = crate::mixed_lock::lock_builder(coins, script.clone())
            .build_unsigned_reserved()
            .map_err(|e| e.to_string())?;
        crate::mixed_lock::check_lock(&dry, coins, &script)?;
        let builder = crate::mixed_lock::lock_builder(coins, script.clone());
        let signed = wallet
            .core()
            .finalize_transaction_with_options(
                builder,
                &[AccountTypePreference::CoinJoin],
                COINJOIN_ACCOUNT,
                keys,
                true,
            )
            .await
            .map_err(|e| e.to_string())?;
        let credit = match crate::mixed_lock::check_lock(signed.transaction(), coins, &script) {
            Ok(credit) => credit,
            Err(why) => {
                // Not sent: the coins it reserved go back at once.
                wallet.core().abandon_transaction(&signed).await;
                return Err(why);
            }
        };
        let tx = signed.transaction().clone();
        let out_point = OutPoint::new(tx.txid(), 0);
        let fee = signed.fee();
        // Tracked before it can be seen anywhere (the architect, М-2). The call blocks on the
        // wallet's lock: off the async workers.
        let manager = wallet.asset_locks().clone();
        let (funding_type, identity_index) = (purpose.funding_type(), purpose.identity_index());
        let registered = tokio::task::spawn_blocking(move || {
            manager.recover_asset_lock_blocking(
                tx,
                credit,
                COINJOIN_ACCOUNT,
                funding_type,
                identity_index,
                out_point,
                None,
            );
        })
        .await;
        if registered.is_err() || !self.tracks_lock(wallet, &out_point).await {
            wallet.core().abandon_transaction(&signed).await;
            return Err("The lock could not be kept before sending: nothing was sent".into());
        }
        match wallet.core().broadcast_finalized_transaction(&signed).await {
            Ok(_) => Ok(MixedLock {
                out_point,
                credit,
                fee,
                relayed: true,
            }),
            Err(platform_wallet::PlatformWalletError::TransactionBroadcastUnconfirmed(_)) => {
                Ok(MixedLock {
                    out_point,
                    credit,
                    fee,
                    relayed: false,
                })
            }
            // Definitely refused by the network: platform-wallet freed the coins.
            Err(e @ platform_wallet::PlatformWalletError::TransactionBroadcast(_)) => {
                self.untrack_unsent_lock(wallet, &out_point).await?;
                Err(e.to_string())
            }
            // Refused before anything was sent (condition 1: the coins freed here too).
            Err(
                e @ (platform_wallet::PlatformWalletError::StaleReservation
                | platform_wallet::PlatformWalletError::InputMidBroadcast { .. }),
            ) => {
                wallet.core().abandon_transaction(&signed).await;
                self.untrack_unsent_lock(wallet, &out_point).await?;
                Err(e.to_string())
            }
            // Anything else may have reached a node: held, never resent (BUG-243).
            Err(_) => Ok(MixedLock {
                out_point,
                credit,
                fee,
                relayed: false,
            }),
        }
    }

    /// Whether platform-wallet tracks the lock `out_point`.
    async fn tracks_lock(&self, wallet: &PlatformWallet, out_point: &OutPoint) -> bool {
        let manager = wallet.wallet_manager().read().await;
        manager
            .get_wallet_info(&wallet.wallet_id())
            .is_some_and(|info| info.tracked_asset_locks.contains_key(out_point))
    }

    /// A lock registered here that definitely did not go: its removal persisted first, then taken
    /// out of the wallet, under one write guard, so nothing resumes or restores it. Left alone if
    /// it moved past what the registration recorded (a proof seen): then it may have gone. Its
    /// credit address stays used (condition 3).
    async fn untrack_unsent_lock(
        &self,
        wallet: &PlatformWallet,
        out_point: &OutPoint,
    ) -> Result<(), String> {
        let store = self
            .persister
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or("The wallet's store is not open")?;
        let mut manager = wallet.wallet_manager().write().await;
        let info = manager
            .get_wallet_info_mut(&wallet.wallet_id())
            .ok_or(tr!("The wallet is not open"))?;
        let unsent = info
            .tracked_asset_locks
            .get(out_point)
            .is_some_and(|lock| lock.proof.is_none());
        if !unsent {
            return Ok(());
        }
        let mut removed = AssetLockChangeSet::default();
        removed.removed.insert(*out_point);
        store
            .store(
                wallet.wallet_id(),
                PlatformWalletChangeSet {
                    asset_locks: Some(removed),
                    ..Default::default()
                },
            )
            .map_err(|e| format!("{e:?}"))?;
        info.tracked_asset_locks.remove(out_point);
        Ok(())
    }

    /// The one-time key a lock for `purpose` credits: the funding account's next unused address,
    /// as platform-wallet takes it (so its resume and its reconstruction find the key the same
    /// way), marked used and its pool persisted before the lock exists. No race: only the job
    /// holding the identity's or the pool's busy flag takes one.
    async fn take_credit_address(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        purpose: MixedLockFor,
    ) -> Result<Address, String> {
        if let MixedLockFor::TopUp { slot } = purpose {
            self.ensure_topup_account(wallet, keys, slot).await?;
        }
        let (address, kind, pools) = {
            let mut manager = wallet.wallet_manager().write().await;
            let (held, info) = manager
                .get_wallet_mut_and_info_mut(&wallet.wallet_id())
                .ok_or(tr!("The wallet is not open"))?;
            // The credit scripts of every lock platform-wallet tracks. A restart restores the
            // used state of funds accounts only (platform-wallet-storage `rehydrate.rs`), not of
            // these keys accounts: until a sync sees the lock's transaction again, «next unused»
            // could hand out a key a lock already credits. The tracked locks are persisted, so
            // they say which keys are taken.
            let credited: std::collections::BTreeSet<key_wallet::dashcore::ScriptBuf> = info
                .tracked_asset_locks
                .values()
                .filter_map(|lock| match &lock.transaction.special_transaction_payload {
                    Some(key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload::AssetLockPayloadType(p)) => Some(p.credit_outputs.clone()),
                    _ => None,
                })
                .flatten()
                .map(|out| out.script_pubkey)
                .collect();
            let accounts = &mut info.core_wallet.accounts;
            let (xpub, managed, kind) = match purpose {
                MixedLockFor::Identity { .. } => (
                    held.accounts
                        .identity_registration
                        .as_ref()
                        .map(|a| a.account_xpub),
                    accounts.identity_registration.as_mut(),
                    AccountType::IdentityRegistration,
                ),
                MixedLockFor::TopUp { slot } => (
                    held.accounts
                        .identity_topup
                        .get(&slot)
                        .map(|a| a.account_xpub),
                    accounts.identity_topup.get_mut(&slot),
                    AccountType::IdentityTopUp {
                        registration_index: slot,
                    },
                ),
                MixedLockFor::Pool => (
                    held.accounts
                        .asset_lock_shielded_address_topup
                        .as_ref()
                        .map(|a| a.account_xpub),
                    accounts.asset_lock_shielded_address_topup.as_mut(),
                    AccountType::AssetLockShieldedAddressTopUp,
                ),
            };
            let managed = managed.ok_or("The lock's funding account is missing")?;
            let mut address = managed.next_address(xpub.as_ref(), false)?;
            // A key a tracked lock already credits is marked used and passed over.
            let mut passed = 0;
            while credited.contains(&address.script_pubkey()) {
                managed.mark_address_used(&address);
                passed += 1;
                if passed > 1_000 {
                    return Err("No free key for the lock".into());
                }
                address = managed.next_address(xpub.as_ref(), false)?;
            }
            managed.mark_address_used(&address);
            let pools: Vec<AccountAddressPoolEntry> = managed
                .managed_account_type()
                .address_pools()
                .into_iter()
                .filter(|pool| !pool.addresses.is_empty())
                .map(|pool| AccountAddressPoolEntry {
                    account_type: kind,
                    pool_type: pool.pool_type,
                    addresses: pool.addresses.values().cloned().collect(),
                })
                .collect();
            (address, kind, pools)
        };
        let _ = kind;
        self.store_changeset(
            wallet,
            PlatformWalletChangeSet {
                account_address_pools: pools,
                ..Default::default()
            },
        )?;
        Ok(address)
    }

    /// The bound top-up account of slot `slot` (`identity_topup[slot]`), made when missing the
    /// way platform-wallet makes it at its first top-up and the refusal account is made here:
    /// its registration and pool persisted first, then both halves of the wallet (the xpub from
    /// the opened keys).
    pub(super) async fn ensure_topup_account(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        slot: u32,
    ) -> Result<(), String> {
        use key_wallet::managed_account::managed_account_collection::ManagedAccountCollection;
        let kind = AccountType::IdentityTopUp {
            registration_index: slot,
        };
        let mut manager = wallet.wallet_manager().write().await;
        let (held, info) = manager
            .get_wallet_mut_and_info_mut(&wallet.wallet_id())
            .ok_or(tr!("The wallet is not open"))?;
        let has_account = held.accounts.identity_topup.contains_key(&slot);
        let has_managed = info.core_wallet.accounts.identity_topup.contains_key(&slot);
        if has_account && has_managed {
            return Ok(());
        }
        if keys.wallet_id != held.wallet_id {
            return Err("The keys are not this wallet's".into());
        }
        let account = match held.accounts.identity_topup.get(&slot) {
            Some(account) => account.clone(),
            None => {
                let path = kind
                    .derivation_path(held.network)
                    .map_err(|e| e.to_string())?;
                let xpub = keys
                    .derive_extended_public_key(&path)
                    .map_err(|e| e.to_string())?;
                key_wallet::Account::new(Some(held.wallet_id), kind, xpub, held.network)
                    .map_err(|e| e.to_string())?
            }
        };
        let mut one = key_wallet::AccountCollection::new();
        one.insert(account.clone()).map_err(|e| e.to_string())?;
        let managed = ManagedAccountCollection::from_account_collection(&one)
            .identity_topup
            .remove(&slot)
            .ok_or("The top-up account's addresses were not made")?;
        let pools = managed
            .managed_account_type()
            .address_pools()
            .into_iter()
            .map(|pool| AccountAddressPoolEntry {
                account_type: kind,
                pool_type: pool.pool_type,
                addresses: pool.addresses.values().cloned().collect(),
            })
            .collect();
        self.store_changeset(
            wallet,
            PlatformWalletChangeSet {
                account_registrations: vec![AccountRegistrationEntry {
                    account_type: kind,
                    account_xpub: account.account_xpub,
                }],
                account_address_pools: pools,
                ..Default::default()
            },
        )?;
        if !has_account {
            held.add_account(kind, Some(account.account_xpub))
                .map_err(|e| e.to_string())?;
        }
        if !has_managed {
            info.core_wallet
                .accounts
                .identity_topup
                .insert(slot, managed);
        }
        Ok(())
    }

    fn store_changeset(
        &self,
        wallet: &PlatformWallet,
        changeset: PlatformWalletChangeSet,
    ) -> Result<(), String> {
        let store = self
            .persister
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or("The wallet's store is not open")?;
        store
            .store(wallet.wallet_id(), changeset)
            .map_err(|e| format!("{e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::super::addresses::tests::{open_on, store_paths};
    use super::*;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{PubkeyHash, TxOut, Txid};

    /// The funding address a test lock credits, and whether it is marked used.
    async fn used(wallet: &PlatformWallet, purpose: MixedLockFor, address: &Address) -> bool {
        let manager = wallet.wallet_manager().read().await;
        let info = manager.get_wallet_info(&wallet.wallet_id()).unwrap();
        let accounts = &info.core_wallet.accounts;
        let managed = match purpose {
            MixedLockFor::Identity { .. } => accounts.identity_registration.as_ref(),
            MixedLockFor::TopUp { slot } => accounts.identity_topup.get(&slot),
            MixedLockFor::Pool => accounts.asset_lock_shielded_address_topup.as_ref(),
        }
        .unwrap();
        managed
            .managed_account_type()
            .address_pools()
            .into_iter()
            .flat_map(|pool| pool.addresses.values())
            .any(|info| {
                info.address == *address
                    && matches!(
                        info.state,
                        key_wallet::managed_account::address_pool::AddressState::Used
                    )
            })
    }

    /// Condition 3 and the architect's answer 1: the credit key is the funding account's next
    /// unused address, marked used at once (the next lock takes another), for each purpose.
    #[tokio::test(flavor = "multi_thread")]
    async fn each_lock_takes_a_fresh_used_address_of_its_account() {
        let paths = store_paths("mixed-lock-address", true);
        let (chain, wallet, keys) = open_on(&paths).await;
        for purpose in [
            MixedLockFor::Identity { slot: 0 },
            MixedLockFor::TopUp { slot: 2 },
            MixedLockFor::Pool,
        ] {
            let first = chain
                .take_credit_address(&wallet, &keys, purpose)
                .await
                .unwrap();
            let second = chain
                .take_credit_address(&wallet, &keys, purpose)
                .await
                .unwrap();
            assert_ne!(first, second, "{purpose:?}: a fresh one each time");
            assert!(
                used(&wallet, purpose, &first).await,
                "{purpose:?}: marked used"
            );
        }
        chain.stop().await;
    }

    /// The architect's answer 3: the bound top-up account of a slot is made when missing and is
    /// there after a restart. A restart does not bring back the used state of these keys
    /// accounts (platform-wallet-storage restores it for funds accounts only), so the key a
    /// registered lock credits is never handed out again: the tracked locks say which are taken.
    #[tokio::test(flavor = "multi_thread")]
    async fn after_a_restart_a_lock_never_takes_a_credited_key_again() {
        let paths = store_paths("mixed-lock-topup", true);
        let (chain, wallet, keys) = open_on(&paths).await;
        let id = wallet.wallet_id();
        let purpose = MixedLockFor::TopUp { slot: 4 };
        let taken = chain
            .take_credit_address(&wallet, &keys, purpose)
            .await
            .unwrap();
        register_to(&wallet, 30, taken.script_pubkey(), purpose).await;
        drop(wallet);
        chain.stop().await;

        let again = Chain::default();
        again
            .open_store_only(&paths)
            .await
            .expect("the store loads");
        let loaded = again.wallet(&id).await.expect("the wallet loads");
        let next = again
            .take_credit_address(&loaded, &keys, purpose)
            .await
            .unwrap();
        assert_ne!(next, taken, "the credited key is passed over");
        assert!(
            used(&loaded, purpose, &next).await,
            "the account is there, its key used"
        );
        again.stop().await;
    }

    fn fake_coin(n: u8) -> Utxo {
        let at = Address::new(
            key_wallet::Network::Testnet,
            key_wallet::dashcore::address::Payload::PubkeyHash(PubkeyHash::from_byte_array(
                [n; 20],
            )),
        );
        let mut u = Utxo::new(
            OutPoint::new(Txid::from_byte_array([n; 32]), 0),
            TxOut {
                value: 1_000_010,
                script_pubkey: at.script_pubkey(),
            },
            at,
            1_570_000,
            false,
        );
        u.is_confirmed = true;
        u
    }

    /// A lock as `mixed_lock` registers it, tracked before any broadcast.
    async fn register(wallet: &PlatformWallet, n: u8) -> OutPoint {
        let script = Address::new(
            key_wallet::Network::Testnet,
            key_wallet::dashcore::address::Payload::PubkeyHash(PubkeyHash::from_byte_array(
                [200; 20],
            )),
        )
        .script_pubkey();
        register_to(wallet, n, script, MixedLockFor::Identity { slot: 0 }).await
    }

    /// The same, crediting `script` for `purpose`.
    async fn register_to(
        wallet: &PlatformWallet,
        n: u8,
        script: key_wallet::dashcore::ScriptBuf,
        purpose: MixedLockFor,
    ) -> OutPoint {
        let coins: Vec<Utxo> = (n..n + 3).map(fake_coin).collect();
        let (tx, _, _) = crate::mixed_lock::lock_builder(&coins, script.clone())
            .build_unsigned_reserved()
            .unwrap();
        let credit = crate::mixed_lock::check_lock(&tx, &coins, &script).unwrap();
        let out_point = OutPoint::new(tx.txid(), 0);
        let manager = wallet.asset_locks().clone();
        tokio::task::spawn_blocking(move || {
            manager.recover_asset_lock_blocking(
                tx,
                credit,
                COINJOIN_ACCOUNT,
                purpose.funding_type(),
                purpose.identity_index(),
                out_point,
                None,
            )
        })
        .await
        .unwrap();
        out_point
    }

    /// Condition 4: a lock that definitely did not go is untracked, its removal persisted, so
    /// after a restart it is neither resumed nor restored; another one, kept, is still there.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_untracked_lock_stays_gone_after_a_restart() {
        let paths = store_paths("mixed-lock-untrack", true);
        let (chain, wallet, _) = open_on(&paths).await;
        let id = wallet.wallet_id();
        let kept = register(&wallet, 10).await;
        let gone = register(&wallet, 20).await;
        assert!(chain.tracks_lock(&wallet, &kept).await && chain.tracks_lock(&wallet, &gone).await);
        chain.untrack_unsent_lock(&wallet, &gone).await.unwrap();
        assert!(
            !chain.tracks_lock(&wallet, &gone).await,
            "untracked at once"
        );
        assert!(chain.tracks_lock(&wallet, &kept).await, "the other stays");
        drop(wallet);
        chain.stop().await;

        let again = Chain::default();
        again
            .open_store_only(&paths)
            .await
            .expect("the store loads");
        let loaded = again.wallet(&id).await.expect("the wallet loads");
        assert!(!again.tracks_lock(&loaded, &gone).await, "not restored");
        assert!(
            again.tracks_lock(&loaded, &kept).await,
            "a kept lock was persisted"
        );
        again.stop().await;
    }

    /// BUG-273 (the architect, for step 2): a mixed-coin lock is refused like any new asset lock
    /// while the wallet's funding keys could not be checked against the store, before a key is
    /// taken or a coin reserved.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_mixed_lock_waits_for_the_funding_keys_to_be_checked() {
        let paths = store_paths("mixed-lock-unchecked", true);
        let (chain, wallet, keys) = open_on(&paths).await;
        let store = chain.persister.lock().unwrap().take();
        chain.mark_funding_keys_used(&wallet).await;
        let coins: Vec<Utxo> = (40..43).map(fake_coin).collect();
        let refused = chain
            .mixed_lock(&wallet, &keys, &coins, MixedLockFor::Identity { slot: 0 })
            .await
            .unwrap_err();
        assert!(refused.contains("could not be checked"), "{refused}");
        *chain.persister.lock().unwrap() = store;
        chain.stop().await;
    }
}
