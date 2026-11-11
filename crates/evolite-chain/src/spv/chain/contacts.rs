//! DashPay on the chain: profiles, contact requests, and payments to contacts.

use super::*;

/// What [`Chain::scan_dashpay`] found for a phrase.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DashPayFound {
    /// The identity (base58), its credits and how many keys it has.
    pub identity: Option<(String, u64, u32)>,
    pub name: Option<String>,
    /// Contacts established both ways.
    pub contacts: u32,
}

impl Chain {
    /// The DashPay profile of any identity (proved), `None` when it has written none.
    pub async fn profile_of(
        &self,
        id: Identifier,
    ) -> Result<Option<platform_wallet::DashPayProfile>, String> {
        self.ready()?;
        let sdk = self.sdk().await?;
        let contract = Arc::new(
            dpp::system_data_contracts::load_system_data_contract(
                dpp::data_contracts::SystemDataContract::Dashpay,
                sdk.version(),
            )
            .map_err(|e| e.to_string())?,
        );
        let docs = sdk
            .get_profiles(&contract, &[id])
            .await
            .map_err(|e| e.to_string())?;
        Ok(docs.get(&id).map(dashpay::profile_from_document))
    }

    /// What a phrase brings from DashPay, read from Platform before any wallet is made:
    /// the first identity of its DIP-13 keys (slots 0 to 4, by the hash of each slot's master
    /// key), its credits and keys, its name, and how many contacts it has established (DIP-15:
    /// requests both ways). All proved. Core balances need the wallet's own sync: not here.
    pub async fn scan_dashpay(&self, keys: &key_wallet::Wallet) -> Result<DashPayFound, String> {
        #[cfg(any(test, feature = "test-support"))]
        self.dashpay_scans.fetch_add(1, Ordering::Relaxed);
        use dash_sdk::platform::types::identity::PublicKeyHash;
        use dash_sdk::platform::{Fetch, FetchMany};
        use dpp::identity::accessors::IdentityGettersV0;
        self.ready()?;
        let sdk = self.sdk().await?;
        let mut identity = None;
        for slot in 0..5 {
            let (_, _, public) = platform_wallet::derive_identity_auth_keypair(
                keys,
                crate::network::dash(),
                slot,
                0,
            )
            .map_err(|e| e.to_string())?;
            let hash = dpp::util::hash::ripemd160_sha256(&public.to_compressed());
            if let Some(found) = dpp::identity::Identity::fetch(&sdk, PublicKeyHash(hash))
                .await
                .map_err(|e| e.to_string())?
            {
                identity = Some(found);
                break;
            }
        }
        let Some(identity) = identity else {
            return Ok(DashPayFound::default());
        };
        let id = identity.id();
        let name = self.name_of(id).await?;
        // Established contacts: a request we sent to them, and one they sent to us.
        let contract = std::sync::Arc::new(
            dpp::system_data_contracts::load_system_data_contract(
                dpp::system_data_contracts::SystemDataContract::Dashpay,
                sdk.version(),
            )
            .map_err(|e| e.to_string())?,
        );
        let requests = |field: &str, order: &str| {
            use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
            dash_sdk::platform::DocumentQuery::new(contract.clone(), "contactRequest")
                .map(|q| {
                    q.with_where(WhereClause {
                        field: field.into(),
                        operator: WhereOperator::Equal,
                        value: dpp::platform_value::Value::Identifier(id.to_buffer()),
                    })
                    .with_order_by(OrderClause {
                        field: order.into(),
                        ascending: true,
                    })
                    .with_limit(100)
                })
                .map_err(|e| e.to_string())
        };
        let sent = requests("$ownerId", "$createdAt")?;
        let got = requests("toUserId", "$createdAt")?;
        let sent = dpp::document::Document::fetch_many(&sdk, sent)
            .await
            .map_err(|e| e.to_string())?;
        let got = dpp::document::Document::fetch_many(&sdk, got)
            .await
            .map_err(|e| e.to_string())?;
        let to: std::collections::BTreeSet<Identifier> = sent
            .into_values()
            .flatten()
            .filter_map(|d| {
                use dpp::document::DocumentV0Getters;
                d.properties().get("toUserId")?.to_identifier().ok()
            })
            .collect();
        let contacts = got
            .into_values()
            .flatten()
            .filter(|d| {
                use dpp::document::DocumentV0Getters;
                to.contains(&d.owner_id())
            })
            .count() as u32;
        Ok(DashPayFound {
            identity: Some((
                id.to_string(Encoding::Base58),
                identity.balance(),
                identity.public_keys().len() as u32,
            )),
            name,
            contacts,
        })
    }

    /// The DPNS name of an identity, if it has one (proved).
    pub async fn name_of(&self, id: Identifier) -> Result<Option<String>, String> {
        self.ready()?;
        let names = self
            .sdk()
            .await?
            .get_dpns_usernames_by_identity(id, Some(1))
            .await
            .map_err(|e| e.to_string())?;
        Ok(names.into_iter().next().map(|n| n.label))
    }

    /// Where `to` stands among the contacts of the identity a request goes from (the shown one,
    /// as [`Self::add_contact`] signs), read from platform-wallet now: the last snapshot may
    /// still be of the identity shown before "Act as" (BUG-13). With the identity it goes from.
    pub async fn contact_standing(
        wallet: &PlatformWallet,
        to: &Identifier,
    ) -> Result<(Identifier, Option<evolite_api::ContactState>), String> {
        let state = wallet.state().await;
        let mine = of_wallet(&state.identity_manager, wallet.wallet_id())
            .filter_map(|m| Some((m.identity_index?, m)));
        let (_, managed) =
            shown_of(wallet.wallet_id(), mine).ok_or(tr!("The wallet has no identity"))?;
        Ok((managed.identity.id(), dashpay::standing(managed, to)))
    }

    /// When platform-wallet's DashPay sync last finished a pass (unix seconds); `None` before
    /// the first one or without a chain. A new pass may have brought requests (BUG-13).
    pub async fn dashpay_synced_at(&self) -> Option<u64> {
        self.manager()
            .await
            .ok()?
            .dashpay_sync_arc()
            .last_sync_unix_seconds()
    }

    /// Sends our DashPay contact request to `to`: an encrypted key to pay us with, signed by
    /// the keys of `from` — the identity [`Self::contact_standing`] checked it as, not whichever
    /// is shown by now (BUG-13 r1 should 1).
    pub async fn add_contact(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        from: &Identifier,
        to: Identifier,
    ) -> Result<(), String> {
        let (from, signer) = Self::identity_as(wallet, keys, from).await?;
        let crypto = dashpay::SeedCrypto(keys.clone());
        wallet
            .identity()
            .dashpay()
            .send_contact_request_with_external_signer(
                &from.id(),
                &to,
                None,
                AutoAcceptProofSource::None,
                &signer,
                &crypto,
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Answers `from`'s request with ours: the contact is made.
    pub async fn accept_contact(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        from: Identifier,
    ) -> Result<(), String> {
        let (me, signer) = Self::first_identity(wallet, keys).await?;
        let request = {
            let state = wallet.state().await;
            state
                .identity_manager
                .managed_identity(&me.id())
                .and_then(|m| m.dashpay().incoming_contact_requests().get(&from).cloned())
                .ok_or(tr!("There is no request from this identity"))?
        };
        let crypto = dashpay::SeedCrypto(keys.clone());
        wallet
            .identity()
            .dashpay()
            .accept_contact_request_with_external_signer(&request, &signer, &crypto)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Stops showing `from`'s requests (kept in the store on this computer, nothing is written
    /// to Platform).
    pub async fn hide_request(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        from: Identifier,
    ) -> Result<(), String> {
        let (me, _) = Self::first_identity(wallet, keys).await?;
        wallet
            .identity()
            .dashpay()
            .ignore_contact_sender(&me.id(), &from)
            .await
            .map_err(|e| e.to_string())
    }

    /// Writes the first identity's DashPay profile, created or replaced. `avatar`: the
    /// picture's link and its bytes (downloaded by the caller), for the hash in the profile.
    pub async fn save_profile(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        display: String,
        about: String,
        avatar: Option<(String, Vec<u8>)>,
    ) -> Result<(), String> {
        let (me, signer) = Self::first_identity(wallet, keys).await?;
        let has_profile = {
            let state = wallet.state().await;
            state
                .identity_manager
                .managed_identity(&me.id())
                .is_some_and(|m| m.dashpay().profile.is_some())
        };
        let (avatar_url, avatar_bytes) = match avatar {
            Some((url, bytes)) => (Some(url), Some(bytes)),
            None => (None, None),
        };
        let input = ProfileUpdate {
            display_name: Some(display).filter(|s| !s.is_empty()),
            public_message: Some(about).filter(|s| !s.is_empty()),
            avatar_url,
            avatar_bytes,
        };
        let view = wallet.identity();
        let view = view.dashpay();
        let written = if has_profile {
            view.update_profile_with_external_signer(&me.id(), input, &signer)
                .await
        } else {
            view.create_profile_with_external_signer(&me.id(), input, &signer)
                .await
        };
        written.map(|_| ()).map_err(|e| e.to_string())
    }

    /// Builds the accounts of the keys every established contact gave us (DIP-15), all at once:
    /// platform-wallet otherwise builds one only for the first payment to that contact in a run,
    /// so a wallet restored from its phrase named no past payment and counted none.
    /// Only platform-wallet's DashPay sweep queues the builds — every established contact
    /// without an account — and its own sweep may not have run yet when the contacts are read
    /// from the store, so it runs here first; then the queue is decrypted with the phrase's
    /// keys. Answers how many of its contacts the identity the interface shows has an account for.
    pub async fn build_contact_accounts(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<usize, String> {
        let (me, _) = Self::first_identity(wallet, keys).await?;
        let view = wallet.identity().dashpay();
        view.sync_contact_requests()
            .await
            .map_err(|e| e.to_string())?;
        let crypto = dashpay::SeedCrypto(keys.clone());
        view.drain_pending_contact_crypto_verified(&crypto, None)
            .await
            .map_err(|e| e.to_string())?;
        let state = wallet.state().await;
        let me = me.id().to_buffer();
        Ok(state
            .core_wallet
            .accounts
            .dashpay_external_accounts
            .keys()
            .filter(|key| key.user_identity_id == me)
            .count())
    }

    /// Every address of the contacts' keys the wallet holds (DIP-15 accounts built from their
    /// requests), with the contact's identity in base58: a payment out to one of them was a
    /// payment to that contact, also after a restore from the phrase.
    pub async fn contact_key_addresses(
        &self,
        wallet: &PlatformWallet,
    ) -> std::collections::HashMap<String, String> {
        use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
        let state = wallet.state().await;
        let mut found = std::collections::HashMap::new();
        for (key, account) in state.core_wallet.accounts.dashpay_external_accounts.iter() {
            let friend = Identifier::from(key.friend_identity_id).to_string(Encoding::Base58);
            for pool in account.managed_account_type().address_pools() {
                for info in pool.addresses.values() {
                    found.insert(info.address.to_string(), friend.clone());
                }
            }
        }
        found
    }

    /// The address the next payment to contact `to` goes to (DIP-15: the first unused address of
    /// the key its request gave us), without taking it: [`Self::pay_contact`] takes this one.
    /// `paid`: addresses of this key paid in earlier runs, marked used first (platform-wallet
    /// rebuilds the contact's account after a restart without them). `None` while the
    /// contact's account has no unused address yet.
    pub async fn contact_address(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: Identifier,
        paid: &[String],
    ) -> Result<Option<String>, String> {
        let (me, _) = Self::first_identity(wallet, keys).await?;
        // The contact's account is built from its request on first use, as a payment does.
        let crypto = dashpay::SeedCrypto(keys.clone());
        wallet
            .identity()
            .dashpay()
            .drain_pending_contact_crypto_verified(&crypto, None)
            .await
            .map_err(|e| e.to_string())?;
        let key = DashpayAccountKey {
            index: 0,
            user_identity_id: me.id().to_buffer(),
            friend_identity_id: to.to_buffer(),
        };
        use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
        if !paid.is_empty() {
            let mut state = wallet.state_mut().await;
            if let Some(account) = state
                .core_wallet
                .accounts
                .dashpay_external_accounts
                .get_mut(&key)
            {
                let known: Vec<Address> = account
                    .managed_account_type()
                    .address_pools()
                    .iter()
                    .flat_map(|pool| pool.addresses.values())
                    .filter(|info| paid.contains(&info.address.to_string()))
                    .map(|info| info.address.clone())
                    .collect();
                for address in &known {
                    account.mark_address_used(address);
                }
            }
        }
        let state = wallet.state().await;
        // The pool's addresses by index: the first available is the one a payment takes.
        Ok(state
            .core_wallet
            .accounts
            .dashpay_external_accounts
            .get(&key)
            .and_then(|account| {
                let pools = account.managed_account_type().address_pools();
                let pool = pools.first()?;
                pool.addresses
                    .values()
                    .find(|info| info.is_available())
                    .map(|info| info.address.to_string())
            }))
    }

    /// Pays a DashPay contact from Core: to the next address of the key its request gave us,
    /// signed with our keys — and only if that is still `shown`, the address the person saw
    /// before signing. Never resent: when the network gives no clear answer the payment may
    /// have gone, and it is reported so (`relayed: false`, txid unknown), not as a failure.
    pub async fn pay_contact(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: Identifier,
        shown: &str,
        paid: &[String],
        amount: u64,
    ) -> Result<Paid, String> {
        let (me, _) = Self::first_identity(wallet, keys).await?;
        if self
            .contact_address(wallet, keys, to, paid)
            .await?
            .as_deref()
            != Some(shown)
        {
            return Err(
                tr!("The contact's address changed after the check — nothing was sent, check the payment again")
                    .into(),
            );
        }
        let crypto = dashpay::SeedCrypto(keys.clone());
        let sent = wallet
            .identity()
            .dashpay()
            .send_payment(
                &me.id(),
                &to,
                amount,
                None,
                &identity::AssetLockSigner(keys.clone()),
                &crypto,
            )
            .await;
        match sent {
            Ok((txid, _, fee)) => Ok(Paid {
                txid: txid.to_string(),
                fee,
                sent: amount,
                relayed: true,
            }),
            Err(platform_wallet::PlatformWalletError::TransactionBroadcastUnconfirmed(_)) => {
                Ok(Paid {
                    txid: String::new(),
                    fee: 0,
                    sent: amount,
                    relayed: false,
                })
            }
            Err(e) => Err(e.to_string()),
        }
    }
}
