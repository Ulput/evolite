//! BUG-273: after a restart, an asset lock's credit key could be handed out again.
//! platform-wallet-storage restores the «used» state on load for the funds accounts only (BIP44,
//! BIP32, CoinJoin; `sqlite/rehydrate.rs`), not for the keys accounts asset locks are credited
//! to: identity registration, identity top-up, the Platform address and the pool. Until a sync
//! saw the old lock's transaction again, «next unused» (platform-wallet's
//! `peek_next_funding_address`, for a creation or top-up from Core) could give the same key to a
//! new lock, and link the two. At every open, before the first sync, the keys the store recorded
//! used, and those the tracked locks credit, are marked used again in memory.

use super::*;
use key_wallet::dashcore::ScriptBuf;
use key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;

impl Chain {
    /// BUG-273: marks used, in memory, every key of the wallet's asset-lock funding accounts the
    /// store recorded used and every key a tracked lock credits. Read only on the store; a key
    /// marked already stays so (idempotent). How many keys it marked now.
    pub(super) async fn mark_funding_keys_used(&self, wallet: &PlatformWallet) -> usize {
        let store = self
            .persister
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|store| store.db_path().to_path_buf());
        let read = match store {
            Some(path) => store_read::used_funding_scripts(&path, &wallet.wallet_id()),
            None => Err("The wallet's store is not open".into()),
        };
        self.apply_funding_keys(wallet, read).await
    }

    /// BUG-273 r1: whether this wallet's funding keys were checked against the store. A
    /// failed read is not «none used» (TEAM.md §8d-4): until one succeeds, no new asset lock
    /// is built for the wallet. Read again here, so the next attempt can pass.
    pub async fn funding_keys_checked(&self, wallet: &PlatformWallet) -> bool {
        if !self.funding_keys_unchecked_now(&wallet.wallet_id()) {
            return true;
        }
        self.mark_funding_keys_used(wallet).await;
        !self.funding_keys_unchecked_now(&wallet.wallet_id())
    }

    /// BUG-273 r1: the refusal of a new asset lock while the wallet's funding keys are unchecked.
    pub(crate) async fn require_funding_keys(&self, wallet: &PlatformWallet) -> Result<(), String> {
        if self.funding_keys_checked(wallet).await {
            Ok(())
        } else {
            Err(tr!("The wallet's keys could not be checked yet — try again in a minute").into())
        }
    }

    fn funding_keys_unchecked_now(&self, id: &WalletId) -> bool {
        self.funding_keys_unchecked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(id)
    }

    /// The marking from a `read` of the store's used keys. A failed read marks the tracked
    /// locks' keys only, sets the wallet «unchecked» and says so in the journal.
    async fn apply_funding_keys(
        &self,
        wallet: &PlatformWallet,
        read: Result<Vec<Vec<u8>>, String>,
    ) -> usize {
        let id = wallet.wallet_id();
        let mut used: std::collections::BTreeSet<ScriptBuf> = match read {
            Ok(scripts) => {
                self.funding_keys_unchecked
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                scripts.into_iter().map(ScriptBuf::from).collect()
            }
            Err(e) => {
                crate::diag::platform_failed("used_funding_scripts", &e);
                self.funding_keys_unchecked
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id);
                Default::default()
            }
        };
        let mut manager = wallet.wallet_manager().write().await;
        let Some(info) = manager.get_wallet_info_mut(&id) else {
            return 0;
        };
        used.extend(credited_scripts(
            info.tracked_asset_locks
                .values()
                .map(|lock| &lock.transaction),
        ));
        let accounts = &mut info.core_wallet.accounts;
        let funding = accounts
            .identity_registration
            .iter_mut()
            .chain(accounts.identity_topup.values_mut())
            .chain(accounts.identity_topup_not_bound.iter_mut())
            .chain(accounts.asset_lock_address_topup.iter_mut())
            .chain(accounts.asset_lock_shielded_address_topup.iter_mut());
        let mut marked = 0;
        for managed in funding {
            let addresses: Vec<_> = managed
                .managed_account_type()
                .address_pools()
                .into_iter()
                .flat_map(|pool| pool.addresses.values())
                .filter(|info| used.contains(&info.script_pubkey))
                .map(|info| info.address.clone())
                .collect();
            for address in addresses {
                if managed.mark_address_used(&address) {
                    marked += 1;
                }
            }
        }
        marked
    }
}

/// The credit outputs' scripts of the asset-lock transactions among `transactions`.
fn credited_scripts<'a>(
    transactions: impl Iterator<Item = &'a key_wallet::dashcore::Transaction>,
) -> Vec<ScriptBuf> {
    transactions
        .filter_map(|tx| match &tx.special_transaction_payload {
            Some(TransactionPayload::AssetLockPayloadType(payload)) => {
                Some(payload.credit_outputs.clone())
            }
            _ => None,
        })
        .flatten()
        .map(|out| out.script_pubkey)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use key_wallet::account::AccountType;
    use platform_wallet::changeset::{
        AccountAddressPoolEntry, PlatformWalletChangeSet, PlatformWalletPersistence,
    };

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn paths(name: &str) -> ChainPaths {
        let dir =
            std::env::temp_dir().join(format!("evolite-bug273-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ChainPaths {
            spv: dir.join("spv").join("testnet"),
            store: dir
                .join("wallets")
                .join("testnet")
                .join("platform-wallet.db"),
            core_node: dir.join("core-node.txt"),
            held_since: dir.join("held-since.json"),
        }
    }

    /// The bound top-up account's slot in the tests.
    const SLOT: u32 = 3;

    /// The funding accounts the test takes keys from, as a lock for each would: a registration,
    /// an identity top-up (bound to slot [`SLOT`], and unbound), a Platform address top-up and
    /// a shield into the pool (r1 Should).
    const KINDS: [AccountType; 5] = [
        AccountType::IdentityRegistration,
        AccountType::IdentityTopUp {
            registration_index: SLOT,
        },
        AccountType::IdentityTopUpNotBoundToIdentity,
        AccountType::AssetLockAddressTopUp,
        AccountType::AssetLockShieldedAddressTopUp,
    ];

    /// The bound top-up account of `slot`, made as platform-wallet makes it at its first top-up
    /// (`ensure_identity_topup_account`): its registration and pool persisted, then both halves.
    async fn add_topup_account(chain: &Chain, wallet: &PlatformWallet, slot: u32) {
        use key_wallet::managed_account::managed_account_collection::ManagedAccountCollection;
        use platform_wallet::changeset::AccountRegistrationEntry;
        let keys = crate::keys_of(PHRASE).unwrap();
        let kind = AccountType::IdentityTopUp {
            registration_index: slot,
        };
        let mut manager = wallet.wallet_manager().write().await;
        let (held, info) = manager
            .get_wallet_mut_and_info_mut(&wallet.wallet_id())
            .unwrap();
        let path = kind.derivation_path(held.network).unwrap();
        let xpub = keys.derive_extended_public_key(&path).unwrap();
        let account =
            key_wallet::Account::new(Some(held.wallet_id), kind, xpub, held.network).unwrap();
        let mut one = key_wallet::AccountCollection::new();
        one.insert(account.clone()).unwrap();
        let managed = ManagedAccountCollection::from_account_collection(&one)
            .identity_topup
            .remove(&slot)
            .unwrap();
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
        let store = chain.persister.lock().unwrap().clone().unwrap();
        store
            .store(
                wallet.wallet_id(),
                PlatformWalletChangeSet {
                    account_registrations: vec![AccountRegistrationEntry {
                        account_type: kind,
                        account_xpub: account.account_xpub,
                    }],
                    account_address_pools: pools,
                    ..Default::default()
                },
            )
            .unwrap();
        held.add_account(kind, Some(account.account_xpub)).unwrap();
        info.core_wallet
            .accounts
            .identity_topup
            .insert(slot, managed);
    }

    /// «Next unused» of a funding account, as platform-wallet's `peek_next_funding_address`
    /// takes it.
    async fn next_key(wallet: &PlatformWallet, kind: AccountType) -> key_wallet::dashcore::Address {
        let mut manager = wallet.wallet_manager().write().await;
        let (held, info) = manager
            .get_wallet_mut_and_info_mut(&wallet.wallet_id())
            .unwrap();
        let accounts = &mut info.core_wallet.accounts;
        let (xpub, managed) = match kind {
            AccountType::IdentityRegistration => (
                held.accounts
                    .identity_registration
                    .as_ref()
                    .map(|a| a.account_xpub),
                accounts.identity_registration.as_mut(),
            ),
            AccountType::IdentityTopUpNotBoundToIdentity => (
                held.accounts
                    .identity_topup_not_bound
                    .as_ref()
                    .map(|a| a.account_xpub),
                accounts.identity_topup_not_bound.as_mut(),
            ),
            AccountType::IdentityTopUp { registration_index } => (
                held.accounts
                    .identity_topup
                    .get(&registration_index)
                    .map(|a| a.account_xpub),
                accounts.identity_topup.get_mut(&registration_index),
            ),
            AccountType::AssetLockShieldedAddressTopUp => (
                held.accounts
                    .asset_lock_shielded_address_topup
                    .as_ref()
                    .map(|a| a.account_xpub),
                accounts.asset_lock_shielded_address_topup.as_mut(),
            ),
            _ => (
                held.accounts
                    .asset_lock_address_topup
                    .as_ref()
                    .map(|a| a.account_xpub),
                accounts.asset_lock_address_topup.as_mut(),
            ),
        };
        managed.unwrap().next_address(xpub.as_ref(), false).unwrap()
    }

    /// A lock built on `key`: platform-wallet marks it used and persists a snapshot of the
    /// funding account's pool (`build.rs`, `persist_asset_lock_account_pools`).
    async fn lock_on(
        chain: &Chain,
        wallet: &PlatformWallet,
        kind: AccountType,
        key: &key_wallet::dashcore::Address,
    ) {
        let entries = {
            let mut manager = wallet.wallet_manager().write().await;
            let info = manager.get_wallet_info_mut(&wallet.wallet_id()).unwrap();
            let accounts = &mut info.core_wallet.accounts;
            let managed = match kind {
                AccountType::IdentityRegistration => accounts.identity_registration.as_mut(),
                AccountType::IdentityTopUpNotBoundToIdentity => {
                    accounts.identity_topup_not_bound.as_mut()
                }
                AccountType::IdentityTopUp { registration_index } => {
                    accounts.identity_topup.get_mut(&registration_index)
                }
                AccountType::AssetLockShieldedAddressTopUp => {
                    accounts.asset_lock_shielded_address_topup.as_mut()
                }
                _ => accounts.asset_lock_address_topup.as_mut(),
            }
            .unwrap();
            assert!(managed.mark_address_used(key));
            managed
                .managed_account_type()
                .address_pools()
                .into_iter()
                .map(|pool| AccountAddressPoolEntry {
                    account_type: kind,
                    pool_type: pool.pool_type,
                    addresses: pool.addresses.values().cloned().collect(),
                })
                .collect::<Vec<_>>()
        };
        let store = chain.persister.lock().unwrap().clone().unwrap();
        store
            .store(
                wallet.wallet_id(),
                PlatformWalletChangeSet {
                    account_address_pools: entries,
                    ..Default::default()
                },
            )
            .unwrap();
    }

    /// BUG-273 (§8d-1, read back after a restart): a key a lock was built on is recorded used in
    /// the store; a wallet loaded from the store alone hands it out again (the bug); opened, it
    /// marks it used, and the next lock of each kind takes another key.
    #[tokio::test(flavor = "multi_thread")]
    async fn after_a_restart_the_next_lock_takes_a_new_key() {
        let paths = paths("restart");
        let seed = crate::seed_from(PHRASE, "").unwrap();
        let chain = Chain::default();
        chain.open_store_only(&paths).await.unwrap();
        let (id, wallet) = chain.open_wallet(&seed, 0).await.unwrap();
        add_topup_account(&chain, &wallet, SLOT).await;
        let mut taken = Vec::new();
        for kind in KINDS {
            let key = next_key(&wallet, kind).await;
            lock_on(&chain, &wallet, kind, &key).await;
            assert_ne!(
                next_key(&wallet, kind).await,
                key,
                "{kind:?}: marked in this run"
            );
            taken.push((kind, key));
        }
        drop(wallet);
        chain.stop().await;

        let again = Chain::default();
        again.open_store_only(&paths).await.unwrap();
        let loaded = again.wallet(&id).await.expect("loaded from the store");
        let store = paths.store.clone();
        let used = store_read::used_funding_scripts(&store, &id).unwrap();
        for (kind, key) in &taken {
            assert!(
                used.contains(&key.script_pubkey().to_bytes()),
                "{kind:?}: the store says used"
            );
            assert_eq!(
                &next_key(&loaded, *kind).await,
                key,
                "{kind:?}: the bug, before the open"
            );
        }
        let (_, opened) = again.open_wallet(&seed, 0).await.unwrap();
        for (kind, key) in &taken {
            assert_ne!(
                &next_key(&opened, *kind).await,
                key,
                "{kind:?}: a new key after the open"
            );
        }
        assert_eq!(
            again.mark_funding_keys_used(&opened).await,
            0,
            "marked once"
        );
        again.stop().await;
    }

    /// A tracked lock's credit is a used key, even with nothing in the store for it; a
    /// transaction without an asset-lock payload credits nothing.
    #[test]
    fn a_tracked_locks_credit_is_a_used_key() {
        use key_wallet::dashcore::blockdata::transaction::special_transaction::asset_lock::AssetLockPayload;
        use key_wallet::dashcore::{Transaction, TxOut};
        let script = ScriptBuf::from(vec![0x76, 0xa9, 0x14, 7, 7, 7]);
        let lock = Transaction {
            version: 3,
            lock_time: 0,
            input: Vec::new(),
            output: Vec::new(),
            special_transaction_payload: Some(TransactionPayload::AssetLockPayloadType(
                AssetLockPayload::new(vec![TxOut {
                    value: 1,
                    script_pubkey: script.clone(),
                }]),
            )),
        };
        let plain = Transaction {
            special_transaction_payload: None,
            ..lock.clone()
        };
        let found = credited_scripts([lock, plain].iter());
        assert_eq!(found, vec![script]);
    }

    /// BUG-273 r1 Must (§8d-4): a read of the store that failed is «unchecked», never «none
    /// used»: a new asset lock — a creation from Core here — is refused with the words, and the
    /// next attempt reads again; once the read succeeds the lock is allowed.
    #[tokio::test(flavor = "multi_thread")]
    async fn unchecked_funding_keys_refuse_a_new_lock_until_a_read_succeeds() {
        let paths = paths("unchecked");
        let seed = crate::seed_from(PHRASE, "").unwrap();
        let chain = Chain::default();
        chain.open_store_only(&paths).await.unwrap();
        let (_, wallet) = chain.open_wallet(&seed, 0).await.unwrap();
        assert!(
            chain.funding_keys_checked(&wallet).await,
            "read at the open"
        );

        // The store cannot be read: taken away.
        let store = chain.persister.lock().unwrap().take();
        chain.mark_funding_keys_used(&wallet).await;
        let keys = crate::keys_of(PHRASE).unwrap();
        let refused = chain
            .create_identity(&wallet, &keys, 0, None, 1_000_000, 0)
            .await
            .unwrap_err();
        assert!(refused.contains("could not be checked"), "{refused}");

        // Readable again: the next attempt reads, and the gate lets it through.
        *chain.persister.lock().unwrap() = store;
        assert!(chain.require_funding_keys(&wallet).await.is_ok());
        assert!(chain.funding_keys_checked(&wallet).await);
        chain.stop().await;
    }
}
