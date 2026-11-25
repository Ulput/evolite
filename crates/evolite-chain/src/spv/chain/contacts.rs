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

/// How many names one search for the suggestions asks for (FEAT-0039, architect §1): the exact
/// name is always first among those starting with it, and the rest leave room for «shorter
/// first» and for the contacts and own identities taken out.
pub const NAMES_PER_SEARCH: u32 = 10;

/// A DPNS name found by [`Chain::search_names`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundName {
    /// As written on the network.
    pub label: String,
    /// DPNS's homograph-safe form.
    pub normalized: String,
    /// The identity it points to, base58.
    pub identity_id: String,
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

    /// The DashPay profiles of several identities in one proved read (FEAT-0039: the names of
    /// the strangers shown in the suggestions), by identity. Those without a profile are left out.
    pub async fn profiles_of(
        &self,
        ids: &[Identifier],
    ) -> Result<HashMap<Identifier, platform_wallet::DashPayProfile>, String> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
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
            .get_profiles(&contract, ids)
            .await
            .map_err(|e| e.to_string())?;
        Ok(docs
            .iter()
            .map(|(id, doc)| (*id, dashpay::profile_from_document(doc)))
            .collect())
    }

    /// DPNS names under `.dash` whose homograph-safe form starts with `prefix` (already in that
    /// form, [`evolite_common::dpns::search_key`]), at most [`NAMES_PER_SEARCH`], in the order of
    /// that form — proved (FEAT-0039 AC-T1). The DPNS contract comes from our context provider.
    /// Contested and blocked names have no domain and are not found.
    pub async fn search_names(&self, prefix: &str) -> Result<Vec<FoundName>, String> {
        self.ready()?;
        let found = self
            .sdk()
            .await?
            .search_dpns_names(prefix, Some(NAMES_PER_SEARCH))
            .await
            .map_err(|e| e.to_string())?;
        Ok(found
            .into_iter()
            .map(|n| FoundName {
                identity_id: n
                    .records_identity_id
                    .unwrap_or(n.owner_id)
                    .to_string(Encoding::Base58),
                normalized: n.normalized_label,
                label: n.label,
            })
            .collect())
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
        let sent = contact_request_query(contract.clone(), "$ownerId", id)?;
        let got = contact_request_query(contract.clone(), "toUserId", id)?;
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
        let (from, signer) = Self::signing(wallet, keys, from).await?;
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
        me: &Identifier,
        from: Identifier,
    ) -> Result<(), String> {
        let (me, signer) = Self::signing(wallet, keys, me).await?;
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
        from: Identifier,
    ) -> Result<(), String> {
        let me = Self::shown_identity(wallet).await?;
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
        me: &Identifier,
        display: String,
        about: String,
        avatar: Option<(String, Vec<u8>)>,
    ) -> Result<(), String> {
        let (me, signer) = Self::signing(wallet, keys, me).await?;
        let has = || async {
            let state = wallet.state().await;
            state
                .identity_manager
                .managed_identity(&me.id())
                .is_some_and(|m| m.dashpay().profile.is_some())
        };
        let view = wallet.identity();
        let view = view.dashpay();
        // BUG-243 (6): «no profile» from the store alone is not enough for a create — the
        // profile's unique index refuses a second one, with a fee. Read from Platform first.
        let mut has_profile = has().await;
        if !has_profile {
            let _ = view.sync_profiles().await;
            has_profile = has().await;
        }
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
        let written = if has_profile {
            view.update_profile_with_external_signer(&me.id(), input, &signer)
                .await
        } else {
            match view
                .create_profile_with_external_signer(&me.id(), input.clone(), &signer)
                .await
            {
                // BUG-243 (6): refused as a second profile — ours landed after all (a read
                // before was a node behind): read again and replace it, never create again.
                Err(e) if profile_exists(&e.to_string()) => {
                    let _ = view.sync_profiles().await;
                    if has().await {
                        view.update_profile_with_external_signer(&me.id(), input, &signer)
                            .await
                    } else {
                        Err(e)
                    }
                }
                other => other,
            }
        };
        written.map(|_| ()).map_err(|e| e.to_string())
    }

    /// BUG-148: at the unlock, before the network, the receiving accounts of our contacts whose
    /// build is queued in the store are built with the phrase's keys, and a contact's coin the
    /// store's early load filed in account 0 goes to its own account: it can be signed for. Answers
    /// how many coins moved.
    pub async fn rehome_contact_coins(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<usize, String> {
        // The coins as the store gave them, before the network (BUG-150: numbers only).
        crate::diag::record(crate::convert::coins_record(
            "unlock",
            &wallet.state().await.core_wallet,
        ));
        let crypto = dashpay::SeedCrypto(keys.clone());
        wallet
            .identity()
            .dashpay()
            .drain_pending_contact_crypto_verified(&crypto, None)
            .await
            .map_err(|e| e.to_string())?;
        let mut state = wallet.state_mut().await;
        let moved = crate::convert::rehome_filed_coins(&mut state.core_wallet);
        // And after the move: where a payment finds the coins from now on.
        crate::diag::record(crate::convert::rehomed_record(&state.core_wallet, moved));
        Ok(moved)
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
        let me = Self::shown_identity(wallet).await?;
        let view = wallet.identity().dashpay();
        view.sync_contact_requests()
            .await
            .map_err(|e| e.to_string())?;
        let crypto = dashpay::SeedCrypto(keys.clone());
        view.drain_pending_contact_crypto_verified(&crypto, None)
            .await
            .map_err(|e| e.to_string())?;
        // A contact's coin filed in account 0 at the store's early load goes to its account now
        // (BUG-148).
        crate::convert::rehome_filed_coins(&mut wallet.state_mut().await.core_wallet);
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
    ) -> std::collections::HashMap<String, (String, String)> {
        use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
        let state = wallet.state().await;
        let mut found = std::collections::HashMap::new();
        for (key, account) in state.core_wallet.accounts.dashpay_external_accounts.iter() {
            // BUG-196: whose account it is too — a wallet's identities may share a contact.
            let ours = Identifier::from(key.user_identity_id).to_string(Encoding::Base58);
            let friend = Identifier::from(key.friend_identity_id).to_string(Encoding::Base58);
            for pool in account.managed_account_type().address_pools() {
                for info in pool.addresses.values() {
                    found.insert(info.address.to_string(), (ours.clone(), friend.clone()));
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
        me: &Identifier,
        to: Identifier,
        paid: &[String],
    ) -> Result<Option<String>, String> {
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
            user_identity_id: me.to_buffer(),
            friend_identity_id: to.to_buffer(),
        };
        let mut state = wallet.state_mut().await;
        Ok(state
            .core_wallet
            .accounts
            .dashpay_external_accounts
            .get_mut(&key)
            .and_then(|account| next_contact_address(account, paid)))
    }

    /// Pays a DashPay contact from Core: to the next address of the key its request gave us,
    /// signed with our keys — and only if that is still `shown`, the address the person saw
    /// before signing. Never resent: when the network gives no clear answer the payment may
    /// have gone, and it is reported so (`relayed: false`, txid unknown), not as a failure.
    #[allow(clippy::too_many_arguments)]
    pub async fn pay_contact(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        me: &Identifier,
        to: Identifier,
        shown: &str,
        paid: &[String],
        amount: u64,
    ) -> Result<Paid, String> {
        // The pair and the identity it counts for: `me`, pinned with the command (BUG-206 r1).
        if self
            .contact_address(wallet, keys, me, to, paid)
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
        // Only final coins and our own change (SPV item 3).
        let signer = identity::AssetLockSigner(keys.clone());
        let sent = self
            .with_final_coins(
                wallet,
                true,
                wallet
                    .identity()
                    .dashpay()
                    .send_payment(me, &to, amount, None, &signer, &crypto),
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

/// The address a payment to a contact takes next (BUG-212, BUG-261 r1): the addresses already
/// paid (`paid`, from `contact-addresses.json`) marked used first, then the first of the pool
/// not used, by index. Nothing is reserved: a payment that never reached the network leaves the
/// same address for the next one, so a contact's wallet, which looks a few addresses ahead,
/// still sees it.
pub fn next_contact_address<A>(account: &mut A, paid: &[String]) -> Option<String>
where
    A: key_wallet::managed_account::managed_account_trait::ManagedAccountTrait,
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
    let pools = account.managed_account_type().address_pools();
    let pool = pools.first()?;
    pool.addresses
        .values()
        .find(|info| info.is_available())
        .map(|info| info.address.to_string())
}

/// One of our contact requests to someone, as Platform proved it (FEAT-0020-resend): the
/// encryption key it was signed with, and its account reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SentRequest {
    pub sender_key: u32,
    pub account_reference: u32,
}

impl SentRequest {
    /// The reference's version: its top 4 bits (platform-wallet's `calculate_account_reference`).
    pub fn version(&self) -> u32 {
        self.account_reference >> 28
    }
}

/// The last version a request's reference may have (4 bits). platform-wallet stops there instead
/// of wrapping, so a request past it has the same reference and is refused by the unique index.
pub const LAST_REQUEST_VERSION: u32 = 15;

/// Whether one of `sent` — in any order: no index orders a pair's requests by date — was signed
/// with our encryption key `key`: a request of ours with that key is already there.
pub fn sent_with(sent: &[SentRequest], key: u32) -> bool {
    sent.iter().any(|r| r.sender_key == key)
}

/// Whether our requests to someone have used up the reference's versions.
pub fn versions_used_up(sent: &[SentRequest]) -> bool {
    sent.iter().any(|r| r.version() >= LAST_REQUEST_VERSION)
}

impl Chain {
    /// Every contact request `owner` sent to `to`, proved (FEAT-0020-resend): DashPay's
    /// `ownerIdUserId` index serves the pair, but not by date, so all of them — a handful, at
    /// most one per version of the reference.
    pub async fn sent_requests_to(
        &self,
        owner: Identifier,
        to: Identifier,
    ) -> Result<Vec<SentRequest>, String> {
        use dash_sdk::platform::FetchMany;
        use dpp::document::DocumentV0Getters;
        self.ready()?;
        let sdk = self.sdk().await?;
        let contract = std::sync::Arc::new(
            dpp::system_data_contracts::load_system_data_contract(
                dpp::system_data_contracts::SystemDataContract::Dashpay,
                sdk.version(),
            )
            .map_err(|e| e.to_string())?,
        );
        let query = sent_to_query(contract, owner, to)?;
        let found = dpp::document::Document::fetch_many(&sdk, query)
            .await
            .map_err(|e| e.to_string())?;
        Ok(found
            .into_values()
            .flatten()
            .filter_map(|d| {
                let number = |field: &str| d.properties().get(field)?.to_integer::<u32>().ok();
                Some(SentRequest {
                    sender_key: number("senderKeyIndex")?,
                    account_reference: number("accountReference")?,
                })
            })
            .collect())
    }

    /// FEAT-0020-resend, from platform-wallet's state now: the key a new request of `me` is
    /// signed with — its choice (`select_own_encryption_key`): the first enabled ECDSA
    /// ENCRYPTION key — and each established contact with the sender key of our latest request
    /// to it (platform-wallet replaces it after a send to an established contact). `None`
    /// without the identity or such a key.
    pub async fn resend_state(
        wallet: &PlatformWallet,
        me: &Identifier,
    ) -> Option<(u32, Vec<(Identifier, u32)>)> {
        use dpp::identity::accessors::IdentityGettersV0;
        use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use dpp::identity::{KeyType, Purpose};
        let state = wallet.state().await;
        let managed = state.identity_manager.managed_identity(me)?;
        let key = managed
            .identity
            .public_keys()
            .values()
            .find(|k| {
                k.purpose() == Purpose::ENCRYPTION
                    && k.key_type() == KeyType::ECDSA_SECP256K1
                    && k.disabled_at().is_none()
            })?
            .id();
        let contacts = managed
            .dashpay()
            .established_contacts()
            .iter()
            .map(|(id, c)| (*id, c.outgoing_request.sender_key_index))
            .collect();
        Some((key, contacts))
    }
}

/// Our `contactRequest` documents to `to` (FEAT-0020-resend): by DashPay's `ownerIdUserId`
/// index, both equal, no order (checked offline, `query_index_tests`).
pub(crate) fn sent_to_query(
    contract: std::sync::Arc<dpp::data_contract::DataContract>,
    owner: Identifier,
    to: Identifier,
) -> Result<dash_sdk::platform::DocumentQuery, String> {
    use dash_sdk::drive::query::{WhereClause, WhereOperator};
    let equal = |field: &str, id: Identifier| WhereClause {
        field: field.into(),
        operator: WhereOperator::Equal,
        value: dpp::platform_value::Value::Identifier(id.to_buffer()),
    };
    dash_sdk::platform::DocumentQuery::new(contract, "contactRequest")
        .map(|q| {
            q.with_where(equal("$ownerId", owner))
                .with_where(equal("toUserId", to))
                .with_limit(100)
        })
        .map_err(|e| e.to_string())
}

/// DashPay `contactRequest` documents with `field` (`$ownerId` or `toUserId`) equal to `id`, by
/// date: DashPay's indexes (checked offline, `query_index_tests`).
pub(crate) fn contact_request_query(
    contract: std::sync::Arc<dpp::data_contract::DataContract>,
    field: &str,
    id: Identifier,
) -> Result<dash_sdk::platform::DocumentQuery, String> {
    use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
    dash_sdk::platform::DocumentQuery::new(contract, "contactRequest")
        .map(|q| {
            q.with_where(WhereClause {
                field: field.into(),
                operator: WhereOperator::Equal,
                value: dpp::platform_value::Value::Identifier(id.to_buffer()),
            })
            .with_order_by(OrderClause {
                field: "$createdAt".into(),
                ascending: true,
            })
            .with_limit(100)
        })
        .map_err(|e| e.to_string())
}

/// BUG-243 (6): whether a profile create was refused because the identity has one already —
/// the document's unique index (`$ownerId`), in Platform's words.
fn profile_exists(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("duplicate") || lower.contains("unique") || lower.contains("already exists")
}

#[cfg(test)]
mod sent_request_tests {
    use super::*;

    /// FEAT-0020-resend, review condition 1: our requests to a contact come in any order (no
    /// index orders them by date); a request with our key now is found wherever it is, and the
    /// versions are used up when any of them is at the last one.
    #[test]
    fn our_requests_are_read_in_any_order() {
        let at = |key, version: u32| SentRequest {
            sender_key: key,
            account_reference: (version << 28) | 0x0123_4567,
        };
        for sent in [
            vec![at(6, 0), at(8, 1), at(10, 2)],
            vec![at(10, 2), at(6, 0), at(8, 1)],
            vec![at(8, 1), at(10, 2), at(6, 0)],
        ] {
            assert!(sent_with(&sent, 10), "{sent:?}");
            assert!(!sent_with(&sent, 12), "{sent:?}");
            assert!(!versions_used_up(&sent));
        }
        assert!(versions_used_up(&[at(4, 3), at(6, 15)]));
        assert_eq!(at(6, 15).version(), LAST_REQUEST_VERSION);
    }
}

#[cfg(test)]
mod next_address_tests {
    use super::*;

    /// A contact's watch account, as a payment to them derives it (keys of another phrase).
    fn their_account() -> key_wallet::managed_account::ManagedCoreFundsAccount {
        let other = crate::keys_of(
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
        )
        .unwrap();
        key_wallet::managed_account::ManagedCoreFundsAccount::from_account(&key_wallet::Account {
            parent_wallet_id: None,
            account_type: key_wallet::account::AccountType::DashpayExternalAccount {
                index: 0,
                user_identity_id: [1; 32],
                friend_identity_id: [2; 32],
            },
            network: key_wallet::Network::Testnet,
            account_xpub: other.accounts.coinjoin_accounts[&0].account_xpub,
            is_watch_only: false,
        })
    }

    /// BUG-261 r1 (as BUG-212): nothing is reserved — a payment refused before the network
    /// (its address not written as paid) leaves the same address for the next look; one the
    /// network answered (written as paid) moves the next look on to the next address.
    #[test]
    fn the_next_address_moves_on_only_once_one_is_paid() {
        let mut account = their_account();
        let first = next_contact_address(&mut account, &[]).expect("an address");
        // Refused before the network: not written as paid.
        let mut again = their_account();
        assert_eq!(
            next_contact_address(&mut again, &[]),
            Some(first.clone()),
            "the same address for the next look"
        );
        // The network answered: written as paid, the next look takes the next one.
        let mut after = their_account();
        let next = next_contact_address(&mut after, std::slice::from_ref(&first)).expect("next");
        assert_ne!(next, first, "a new address once the first is paid");
    }
}
