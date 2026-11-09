//! The network side of app sign-in ([`crate::app_connect`]): the app's contract, the identity
//! that signs in with its keys as Platform has them now, the `loginKeyResponse` document, and
//! the IdentityUpdates that add or disable an app's keys.

use std::sync::Arc;

use dpp::identity::accessors::IdentityGettersV0;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use platform_wallet::PlatformWallet;
use simple_signer::signer::SimpleSigner;
use zeroize::Zeroizing;

use crate::app_connect::{self, AppKeys};
use crate::chain::Chain;
use crate::identity;

/// The identity an app signs in as: the shown one, with its keys as Platform has them now.
pub struct SignInIdentity {
    pub id: Identifier,
    pub keys: Vec<IdentityPublicKey>,
    /// Its MASTER private key, from the phrase: the login keys come from it.
    pub master: Zeroizing<[u8; 32]>,
    /// Every key of it this phrase reproduces, with its private half (for signing).
    found: Vec<(IdentityPublicKey, [u8; 32])>,
}

impl SignInIdentity {
    pub fn id_bytes(&self) -> [u8; 32] {
        self.id.to_buffer()
    }

    /// The id the next added key takes.
    pub fn next_key_id(&self) -> u32 {
        self.keys.iter().map(|k| k.id() + 1).max().unwrap_or(0)
    }

    /// A signer of only the key of `purpose` and `level` this phrase reproduces (ECDSA).
    fn signer(
        &self,
        purpose: Purpose,
        level: SecurityLevel,
    ) -> Option<(IdentityPublicKey, SimpleSigner)> {
        let (key, private) = self.found.iter().find(|(k, _)| {
            k.purpose() == purpose
                && k.security_level() == level
                && k.key_type() == KeyType::ECDSA_SECP256K1
                && k.disabled_at().is_none()
        })?;
        let mut signer = SimpleSigner::default();
        signer.add_identity_public_keys([(key.clone(), *private)]);
        Some((key.clone(), signer))
    }
}

impl Drop for SignInIdentity {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for (_, private) in &mut self.found {
            private.zeroize();
        }
    }
}

impl Chain {
    /// The owner of contract `id` (proved); `None` when Platform has no such contract.
    pub async fn contract_owner(&self, id: Identifier) -> Result<Option<Identifier>, String> {
        use dash_sdk::platform::Fetch;
        use dpp::data_contract::DataContract;
        use dpp::data_contract::accessors::v0::DataContractV0Getters;
        let sdk = self.sdk().await?;
        Ok(DataContract::fetch(&sdk, id)
            .await
            .map_err(|e| e.to_string())?
            .map(|c| c.owner_id()))
    }

    /// The shown identity, read from Platform again (an app may have added keys elsewhere).
    pub async fn sign_in_identity(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<SignInIdentity, String> {
        let (shown, _) = Self::first_identity(wallet, keys).await?;
        let _ = wallet.identity().refresh_identity(&shown.id()).await;
        let (identity, slot) = {
            let state = wallet.state().await;
            let managed = state
                .identity_manager
                .managed_identity(&shown.id())
                .ok_or(tr!("The wallet has no identity"))?;
            (
                managed.identity.clone(),
                managed
                    .identity_index
                    .ok_or(tr!("The wallet has no identity"))?,
            )
        };
        let found = identity::found_identity_keys(keys, slot, &identity);
        let master = found
            .iter()
            .find(|(k, _)| {
                k.purpose() == Purpose::AUTHENTICATION
                    && k.security_level() == SecurityLevel::MASTER
                    && k.disabled_at().is_none()
            })
            .map(|(_, private)| Zeroizing::new(*private))
            .ok_or(
                tr!("This identity's MASTER key is not from your phrase: you cannot sign in to apps as it from here"),
            )?;
        Ok(SignInIdentity {
            id: identity.id(),
            keys: identity.public_keys().values().cloned().collect(),
            master,
            found,
        })
    }

    /// Publishes the answer to a sign-in on the key-exchange contract, signed with the
    /// identity's HIGH key: a new document, or ours for this app replaced (one per identity and
    /// app, a unique index).
    #[allow(
        clippy::too_many_arguments,
        reason = "the fields of the key-exchange response document, each a separate value the caller already has"
    )]
    pub async fn publish_login_response(
        &self,
        wallet: &PlatformWallet,
        who: &SignInIdentity,
        app: &[u8; 32],
        request_hash: [u8; 20],
        wallet_ephemeral: [u8; 33],
        payload: [u8; app_connect::PAYLOAD_LEN],
        generation: u32,
    ) -> Result<(), String> {
        let contract = Identifier::from_string(app_connect::CONTRACT, Encoding::Base58)
            .map_err(|e| e.to_string())?;
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let properties = serde_json::json!({
            "contractId": Identifier::from(*app).to_string(Encoding::Base58),
            "appEphemeralPubKeyHash": hex(&request_hash),
            "walletEphemeralPubKey": hex(&wallet_ephemeral),
            "encryptedPayload": hex(&payload),
            "keyIndex": generation,
        })
        .to_string();
        let (key, signer) = who
            .signer(Purpose::AUTHENTICATION, SecurityLevel::HIGH)
            .ok_or(tr!("The identity has no HIGH key from your phrase: there is nothing to sign the answer to the app with"))?;
        let documents = wallet.identity();
        match self.own_login_response(who.id, app).await? {
            Some(existing) => documents
                .replace_document_with_signer(
                    &who.id,
                    &contract,
                    app_connect::DOCUMENT_TYPE,
                    &existing,
                    &properties,
                    key.id(),
                    &signer,
                )
                .await
                .map(|_| ()),
            None => documents
                .create_document_with_signer(
                    &who.id,
                    &contract,
                    app_connect::DOCUMENT_TYPE,
                    &properties,
                    None,
                    &signer,
                )
                .await
                .map(|_| ()),
        }
        .map_err(|e| e.to_string())
    }

    /// Our `loginKeyResponse` for `app`, if one is there (proved).
    async fn own_login_response(
        &self,
        owner: Identifier,
        app: &[u8; 32],
    ) -> Result<Option<Identifier>, String> {
        use dash_sdk::drive::query::{WhereClause, WhereOperator};
        use dash_sdk::platform::{DocumentQuery, Fetch};
        use dpp::data_contract::DataContract;
        use dpp::document::{Document, DocumentV0Getters};
        use dpp::platform_value::Value;
        let sdk = self.sdk().await?;
        let id = Identifier::from_string(app_connect::CONTRACT, Encoding::Base58)
            .map_err(|e| e.to_string())?;
        let contract = Arc::new(
            DataContract::fetch(&sdk, id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or(tr!("The app sign-in contract was not found on the network"))?,
        );
        let query = DocumentQuery::new(contract, app_connect::DOCUMENT_TYPE)
            .map_err(|e| e.to_string())?
            .with_where(WhereClause {
                field: "$ownerId".into(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(owner.to_buffer()),
            })
            .with_where(WhereClause {
                field: "contractId".into(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(*app),
            });
        Ok(Document::fetch(&sdk, query)
            .await
            .map_err(|e| e.to_string())?
            .map(|d| d.id()))
    }

    /// Adds the app's keys the identity does not have yet, in one MASTER-signed update, and
    /// answers their ids (authentication, encryption) as Platform has them.
    pub async fn add_app_keys(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        who: &SignInIdentity,
        app_keys: &AppKeys,
    ) -> Result<(u32, u32), String> {
        if let app_connect::OnIdentity::Active { auth, encryption } =
            app_connect::on_identity(app_keys, who.keys.iter())
        {
            return Ok((auth, encryption));
        }
        let next = who.next_key_id();
        let [auth, encryption] = app_keys.identity_keys(next, next + 1);
        let have = |data: &[u8]| who.keys.iter().any(|k| k.data().as_slice() == data);
        let add: Vec<IdentityPublicKey> =
            [(auth, *app_keys.auth), (encryption, *app_keys.encryption)]
                .into_iter()
                .filter(|(k, _)| !have(k.data().as_slice()))
                .map(|(k, _)| k)
                .collect();
        // The MASTER key signs the update; each new key proves itself with its own.
        let (_, mut signer) = who
            .signer(Purpose::AUTHENTICATION, SecurityLevel::MASTER)
            .ok_or(tr!("This identity's MASTER key is not from your phrase"))?;
        let [auth_key, encryption_key] = app_keys.identity_keys(next, next + 1);
        signer.add_identity_public_keys([
            (auth_key, *app_keys.auth),
            (encryption_key, *app_keys.encryption),
        ]);
        wallet
            .identity()
            .update_identity_with_external_signer(&who.id, add, Vec::new(), &signer, None)
            .await
            .map_err(|e| e.to_string())?;
        let fresh = self.sign_in_identity(wallet, keys).await?;
        match app_connect::on_identity(app_keys, fresh.keys.iter()) {
            app_connect::OnIdentity::Active { auth, encryption } => Ok((auth, encryption)),
            _ => Err(tr!(
                "The keys were handed to the network, but they are not on the identity yet"
            )
            .into()),
        }
    }

    /// Disables the keys `ids` of the identity, MASTER-signed: an app that held them cannot
    /// sign with them any more.
    pub async fn disable_keys(
        &self,
        wallet: &PlatformWallet,
        who: &SignInIdentity,
        ids: Vec<u32>,
    ) -> Result<(), String> {
        let (_, signer) = who
            .signer(Purpose::AUTHENTICATION, SecurityLevel::MASTER)
            .ok_or(tr!("This identity's MASTER key is not from your phrase"))?;
        wallet
            .identity()
            .update_identity_with_external_signer(&who.id, Vec::new(), ids, &signer, None)
            .await
            .map_err(|e| e.to_string())?;
        let _ = wallet.identity().refresh_identity(&who.id).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncStatus;
    use std::time::{Duration, Instant};

    /// Wallet B of the test plan: its phrase is the first line of the file named by
    /// `EVOLITE_APP_TEST_IDENTITY` in `.env.test.local`, read here and never printed.
    fn wallet_b_phrase() -> Option<Zeroizing<String>> {
        let path = crate::test_env("EVOLITE_APP_TEST_IDENTITY")?;
        let text = Zeroizing::new(std::fs::read_to_string(path.as_str()).ok()?);
        Some(Zeroizing::new(text.lines().next()?.trim().to_owned()))
    }

    /// The whole sign-in on testnet with wallet B, this test playing the app: the wallet answers
    /// a `dash-key:` request, the app opens the response and asks for its keys with a `dash-st:`
    /// IdentityUpdate, the wallet adds them, "Disconnect" disables them and the next sign-in
    /// takes the next generation. Spends a document write and two identity updates of credits.
    /// `cargo test -p evolite-chain live_app_sign_in -- --ignored --nocapture`
    #[test]
    #[ignore = "needs testnet access and EVOLITE_APP_TEST_IDENTITY in .env.test.local"]
    fn live_app_sign_in_round_trip() {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_stack_size(8 << 20)
                    .build()
                    .unwrap()
                    .block_on(sign_in_round_trip())
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn sign_in_round_trip() {
        use crate::chain::ChainPaths;
        let phrase = wallet_b_phrase().expect("wallet B's phrase");
        let dir = std::env::temp_dir().join("evolite-live-apps");
        let paths = ChainPaths {
            // Headers and masternode lists from the other live tests: not fetched again.
            spv: std::env::temp_dir()
                .join("evolite-live-send")
                .join("spv")
                .join("testnet"),
            store: dir.join("platform-wallet.db"),
            core_node: dir.join("core-node.txt"),
        };
        let chain = Chain::default();
        chain
            .start(&paths, Arc::new(|_: &SyncStatus| {}))
            .await
            .expect("chain starts");
        let (_, wallet) = chain.open_wallet(&phrase, 0).await.expect("wallet opens");
        let keys = crate::keys_of(&phrase).expect("keys");
        let started = Instant::now();
        while !chain.platform_ready() {
            assert!(
                started.elapsed() < Duration::from_secs(40 * 60),
                "not ready"
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        chain
            .discover_identities(&wallet, &keys)
            .await
            .expect("identities found");
        let who = chain
            .sign_in_identity(&wallet, &keys)
            .await
            .expect("an identity");
        let me = who.id_bytes();
        println!(
            "identity {} with {} keys, {:?} credits",
            who.id.to_string(Encoding::Base58),
            who.keys.len(),
            chain.first_identity_credits(&wallet, true).await
        );

        // The app: a fresh ephemeral key and its contract (Jembe's NFT contract, which exists).
        let app_secret = dpp::secp256k1::SecretKey::from_secret_bytes(rand_bytes()).unwrap();
        let app_point = dpp::secp256k1::PublicKey::from_secret_key(&app_secret);
        let app = Identifier::from_string(crate::nfts::CONTRACT, Encoding::Base58)
            .unwrap()
            .to_buffer();
        assert!(
            chain
                .contract_owner(Identifier::from(app))
                .await
                .unwrap()
                .is_some(),
            "the app's contract is found by proof"
        );

        // The wallet answers the sign-in.
        let keys_of = |generation| {
            let login = app_connect::login_key(&who.master, &me, &app, generation);
            AppKeys::of(&login, &me)
        };
        let generation = app_connect::current_generation(keys_of, &who.keys);
        let login = app_connect::login_key(&who.master, &me, &app, generation);
        let wallet_secret = dpp::secp256k1::SecretKey::from_secret_bytes(rand_bytes()).unwrap();
        let nonce: [u8; 12] = rand_bytes::<12>();
        let payload = app_connect::seal(&login, &wallet_secret, &app_point, nonce);
        let request_hash = app_connect::request_hash(&app_point.serialize());
        chain
            .publish_login_response(
                &wallet,
                &who,
                &app,
                request_hash,
                dpp::secp256k1::PublicKey::from_secret_key(&wallet_secret).serialize(),
                payload,
                generation,
            )
            .await
            .expect("the response is published");
        println!("response published, generation {generation}");

        // The app finds the response by (its contract, its request) and opens it.
        let (wallet_point, found) = app_reads_response(&chain, &app, &request_hash).await;
        let opened = app_connect::open(&found, &app_secret, &wallet_point).expect("opens");
        assert_eq!(*opened, *login, "the app has the login key");

        // The app asks for its keys; the wallet checks the transition and adds its own.
        let app_keys = AppKeys::of(&opened, &me);
        let [auth, encryption] = app_keys.identity_keys(who.next_key_id(), who.next_key_id() + 1);
        let asked = app_connect::read_transition(&unsigned_update(me, vec![auth, encryption]))
            .expect("a key registration");
        assert!(asked.adds_only(&keys_of(generation)));
        let (auth_id, encryption_id) = chain
            .add_app_keys(&wallet, &keys, &who, &keys_of(generation))
            .await
            .expect("the keys are added");
        println!("keys #{auth_id} and #{encryption_id} added");

        // "Disconnect": both disabled, and the next sign-in is a new generation.
        let who = chain.sign_in_identity(&wallet, &keys).await.unwrap();
        chain
            .disable_keys(&wallet, &who, vec![auth_id, encryption_id])
            .await
            .expect("the keys are disabled");
        let who = chain.sign_in_identity(&wallet, &keys).await.unwrap();
        assert_eq!(
            app_connect::on_identity(&keys_of(generation), who.keys.iter()),
            app_connect::OnIdentity::Disabled
        );
        assert_eq!(
            app_connect::current_generation(keys_of, &who.keys),
            generation + 1
        );
        println!(
            "disconnected; credits now {:?}",
            chain.first_identity_credits(&wallet, true).await
        );
        chain.stop().await;
    }

    fn rand_bytes<const N: usize>() -> [u8; N] {
        let mut bytes = [0u8; N];
        getrandom::fill(&mut bytes).unwrap();
        bytes
    }

    /// What the app does: the response to its request, proved.
    async fn app_reads_response(
        chain: &Chain,
        app: &[u8; 32],
        request_hash: &[u8; 20],
    ) -> (dpp::secp256k1::PublicKey, [u8; app_connect::PAYLOAD_LEN]) {
        use dash_sdk::drive::query::{WhereClause, WhereOperator};
        use dash_sdk::platform::{DocumentQuery, Fetch};
        use dpp::data_contract::DataContract;
        use dpp::document::{Document, DocumentV0Getters};
        use dpp::platform_value::Value;
        let sdk = chain.sdk().await.unwrap();
        let id = Identifier::from_string(app_connect::CONTRACT, Encoding::Base58).unwrap();
        let contract = Arc::new(DataContract::fetch(&sdk, id).await.unwrap().unwrap());
        let query = DocumentQuery::new(contract, app_connect::DOCUMENT_TYPE)
            .unwrap()
            .with_where(WhereClause {
                field: "contractId".into(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(*app),
            })
            .with_where(WhereClause {
                field: "appEphemeralPubKeyHash".into(),
                operator: WhereOperator::Equal,
                value: Value::Bytes20(*request_hash),
            });
        let doc = Document::fetch(&sdk, query)
            .await
            .unwrap()
            .expect("the response");
        let bytes = |k: &str| doc.properties().get(k).unwrap().to_binary_bytes().unwrap();
        let point = dpp::secp256k1::PublicKey::from_slice(&bytes("walletEphemeralPubKey")).unwrap();
        (point, bytes("encryptedPayload").try_into().unwrap())
    }

    /// The app's `dash-st:` bytes: an IdentityUpdate adding `keys`, unsigned, bare.
    fn unsigned_update(identity: [u8; 32], keys: Vec<IdentityPublicKey>) -> Vec<u8> {
        use dpp::serialization::PlatformSerializable;
        use dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
        use dpp::state_transition::identity_update_transition::v0::IdentityUpdateTransitionV0;
        IdentityUpdateTransition::V0(IdentityUpdateTransitionV0 {
            identity_id: identity.into(),
            revision: 1,
            nonce: 1,
            add_public_keys: keys.into_iter().map(Into::into).collect(),
            disable_public_keys: Vec::new(),
            user_fee_increase: 0,
            signature_public_key_id: 0,
            signature: Default::default(),
        })
        .serialize_to_bytes()
        .unwrap()
    }
}
