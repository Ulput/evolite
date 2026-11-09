//! App sign-in in the core's loop ([`crate::real::app_connect`]): a link becomes a request shown to the
//! person, "Allow" or "Sign" does the work in the background, "Disconnect" takes the
//! app's keys back. Nothing is signed for an app before its request has been shown and approved.

use std::sync::atomic::Ordering;

use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use evolite_api::*;

use crate::real::app_connect::{self, AppKeys, Link, LinkError, OnIdentity};
use crate::real::connected_apps::App;
use crate::real::{AppJob, Real, dashpay_failure, loaded, now};

/// What an app's keys can sign: an unbound HIGH key signs documents of any contract (the fees
/// from the identity's credits), never a transfer.
const ANY_CONTRACT: &str = tr!("any contracts");

fn base58(bytes: &[u8; 32]) -> String {
    Identifier::from(*bytes).to_string(Encoding::Base58)
}

/// A fresh secret key from the system's generator.
fn fresh_secret() -> dpp::secp256k1::SecretKey {
    loop {
        let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(bytes.as_mut()).expect("the OS random generator is available");
        if let Ok(key) = dpp::secp256k1::SecretKey::from_secret_bytes(*bytes) {
            return key;
        }
    }
}

/// The base58 contract id of a stored app as bytes.
fn contract_bytes(app: &App) -> Result<[u8; 32], CoreError> {
    Identifier::from_string(&app.contract, Encoding::Base58)
        .map(|c| c.to_buffer())
        .map_err(|e| CoreError::Failed(e.to_string()))
}

/// A request evolite no longer has, or one shown for another identity than it would answer as.
fn out_of_date() -> CoreError {
    CoreError::NotAvailable(tr!("The request is out of date: open the app's link again").into())
}

impl AppJob {
    /// The identity (base58) the request was shown for: «Sign in as», or the app's own.
    fn shown_for(&self) -> String {
        match self {
            AppJob::SignIn { identity, .. } => base58(identity),
            AppJob::AddKeys { app } => app.identity.clone(),
        }
    }

    /// BUG-29 (the architect's hardening): «Allow» answers as the identity the request showed.
    /// The request is shown with «Work as» as it was then and answered with «Work as» as it is
    /// now (`sign_in_identity`); if it was changed in between, the request is out of date
    /// rather than answered by an identity the person did not see.
    fn still_for(&self, me: &[u8; 32]) -> Result<(), CoreError> {
        if self.shown_for() == base58(me) {
            Ok(())
        } else {
            Err(out_of_date())
        }
    }
}

/// The apps signed in as `identity` (base58), as the screens show them.
pub(in crate::real) fn shown_apps(apps: &[App], identity: Option<&str>) -> Vec<ConnectedApp> {
    apps.iter()
        .filter(|a| Some(a.identity.as_str()) == identity)
        .map(|a| ConnectedApp {
            id: a.contract.clone(),
            name: a.name.clone(),
            domain: String::new(),
            key: a.keys.map(|(auth, _)| (auth, KeyLevel::High, Vec::new())),
            last_used: a.last_used,
        })
        .collect()
}

impl Real {
    /// Opens an app's link: checks it, asks Platform what is needed to show it, and answers
    /// `AppRequest` — at once when the link itself is refused, else from the background.
    pub(in crate::real) fn open_app_link(&self, text: &str) -> Result<(), CoreError> {
        let link = app_connect::parse(text).map_err(|e| match e {
            LinkError::NotOurs => CoreError::BadLink,
            LinkError::Refused(why) => CoreError::NotAvailable(why.into()),
        })?;
        let (id, keys) = self.dashpay_ready()?;
        let (chain, requests, apps, fees) = (
            self.chain.clone(),
            self.app_requests.clone(),
            self.apps.clone(),
            crate::real::convert::fees(0),
        );
        let request_id = self.next_app_request.fetch_add(1, Ordering::Relaxed) + 1;
        self.answer_later("task:open_app_link", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let who = chain
                    .sign_in_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                let me = base58(&who.id_bytes());
                let identity = match chain.name_of(who.id).await.ok().flatten() {
                    Some(name) => format!("@{name}"),
                    None => me.clone(),
                };
                let credits = crate::real::identity::credits_as_duffs(
                    chain
                        .first_identity_credits(&wallet, false)
                        .await
                        .unwrap_or(0),
                );
                match link {
                    Link::Key(request) => {
                        let contract = Identifier::from(request.contract_id);
                        let owner = chain
                            .contract_owner(contract)
                            .await
                            .map_err(CoreError::Failed)?;
                        let owner_name = match owner {
                            Some(owner) => chain.name_of(owner).await.ok().flatten(),
                            None => None,
                        };
                        // The brand comes from Platform; the app's own words only in quotes.
                        let name = owner_name
                            .map(|n| format!("@{n}"))
                            .unwrap_or_else(|| format!("«{}»", request.label));
                        let kind = AppRequestKind::Connect {
                            contract_id: base58(&request.contract_id),
                            contract_found: owner.is_some(),
                            identity,
                            key_id: who.next_key_id(),
                            level: KeyLevel::High,
                            contract_name: request.label.clone(),
                            cost: fees.document_write,
                        };
                        requests.lock().unwrap_or_else(|e| e.into_inner()).insert(
                            request_id,
                            AppJob::SignIn {
                                request,
                                name: name.clone(),
                                identity: who.id_bytes(),
                            },
                        );
                        Ok(AppRequest {
                            id: request_id,
                            app: name,
                            kind,
                        })
                    }
                    Link::Transition(bytes) => {
                        let asked = app_connect::read_transition(&bytes)
                            .map_err(|why| CoreError::NotAvailable(why.into()))?;
                        if asked.identity_id != who.id_bytes() {
                            return Err(CoreError::NotAvailable(
                                tr!("The operation is for another identity, not the one chosen in evolite")
                                    .into(),
                            ));
                        }
                        // Only the keys of a sign-in this wallet answered for this identity.
                        let app = apps
                            .all()
                            .into_iter()
                            .filter(|a| a.identity == me)
                            .find(|a| {
                                let Ok(contract) = contract_bytes(a) else {
                                    return false;
                                };
                                let login = app_connect::login_key(
                                    &who.master,
                                    &who.id_bytes(),
                                    &contract,
                                    a.generation,
                                );
                                asked.adds_only(&AppKeys::of(&login, &who.id_bytes()))
                            })
                            .ok_or(CoreError::NotAvailable(
                                tr!("These are not the keys of a sign-in you allowed: first open this app's dash-key: link in evolite").into(),
                            ))?;
                        let next = who.next_key_id();
                        let mut fields = Vec::new();
                        for (n, (purpose, level, kind, _)) in asked.added.iter().enumerate() {
                            let what = match purpose {
                                dpp::identity::Purpose::AUTHENTICATION => {
                                    tr!("sign-in and writing documents")
                                }
                                _ => tr!("message encryption"),
                            };
                            fields.push((
                                format!(tr!("New key #{}"), next + n as u32),
                                format!("{what} · {level:?} · {kind:?} · {ANY_CONTRACT}"),
                            ));
                        }
                        fields.push((
                            tr!("Signs").into(),
                            tr!("evolite: its own identity update with these keys, with the MASTER key")
                                .into(),
                        ));
                        let cost = fees.add_key * asked.added.len() as Duffs;
                        let kind = AppRequestKind::Sign {
                            action: tr!("Add the app's sign-in keys").into(),
                            contract: app.name.clone(),
                            contract_id: app.contract.clone(),
                            document_type: "IdentityUpdate".into(),
                            identity,
                            key_id: 0,
                            level: KeyLevel::Master,
                            fields,
                            cost,
                            credits_after: credits - cost,
                            bytes_hex: bytes.iter().map(|b| format!("{b:02x}")).collect(),
                        };
                        let name = app.name.clone();
                        requests
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(request_id, AppJob::AddKeys { app });
                        Ok(AppRequest {
                            id: request_id,
                            app: name,
                            kind,
                        })
                    }
                }
            }
            .await;
            Event::AppRequest(result)
        });
        Ok(())
    }

    /// The person's answer to request `id`: declined at once, approved in the background.
    /// `Ok(true)`: the answer comes later as `Done { what: DoneWhat::App }`.
    pub(in crate::real) fn answer_app(&self, id: u64, approve: bool) -> Result<bool, CoreError> {
        let job = self
            .app_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if !approve {
            return Ok(false);
        }
        let job = job.ok_or_else(out_of_date)?;
        let (wallet_id, keys) = self.dashpay_ready()?;
        let (chain, apps, refresh) = (self.chain.clone(), self.apps.clone(), self.refresher());
        self.answer_later("task:answer_app", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                let who = chain
                    .sign_in_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                let me = who.id_bytes();
                job.still_for(&me)?;
                let keys_of = |contract: &[u8; 32], generation| {
                    let login = app_connect::login_key(&who.master, &me, contract, generation);
                    AppKeys::of(&login, &me)
                };
                match job {
                    AppJob::SignIn { request, name, .. } => {
                        let contract = request.contract_id;
                        let generation =
                            app_connect::current_generation(|g| keys_of(&contract, g), &who.keys);
                        let login = app_connect::login_key(&who.master, &me, &contract, generation);
                        let ephemeral = fresh_secret();
                        let mut nonce = [0u8; 12];
                        getrandom::fill(&mut nonce).expect("the OS random generator is available");
                        let app_point =
                            dpp::secp256k1::PublicKey::from_slice(&request.app_ephemeral)
                                .map_err(|e| CoreError::Failed(e.to_string()))?;
                        let payload = app_connect::seal(&login, &ephemeral, &app_point, nonce);
                        chain
                            .publish_login_response(
                                &wallet,
                                &who,
                                &contract,
                                app_connect::request_hash(&request.app_ephemeral),
                                dpp::secp256k1::PublicKey::from_secret_key(&ephemeral).serialize(),
                                payload,
                                generation,
                            )
                            .await
                            .map_err(dashpay_failure)?;
                        let keys = match app_connect::on_identity(
                            &keys_of(&contract, generation),
                            who.keys.iter(),
                        ) {
                            OnIdentity::Active { auth, encryption } => Some((auth, encryption)),
                            _ => None,
                        };
                        apps.put(App {
                            contract: base58(&contract),
                            name,
                            identity: base58(&me),
                            generation,
                            keys,
                            last_used: now(),
                        })
                        .map_err(CoreError::Failed)
                    }
                    AppJob::AddKeys { app } => {
                        let app_keys = keys_of(&contract_bytes(&app)?, app.generation);
                        if app_connect::on_identity(&app_keys, who.keys.iter())
                            == OnIdentity::Disabled
                        {
                            return Err(CoreError::NotAvailable(
                                tr!("These keys are already disabled: sign in to the app again with a dash-key: link").into(),
                            ));
                        }
                        let ids = chain
                            .add_app_keys(&wallet, &keys, &who, &app_keys)
                            .await
                            .map_err(dashpay_failure)?;
                        apps.put(App {
                            keys: Some(ids),
                            last_used: now(),
                            ..app
                        })
                        .map_err(CoreError::Failed)
                    }
                }
            }
            .await;
            refresh();
            Event::Done {
                what: DoneWhat::App,
                result,
            }
        });
        Ok(true)
    }

    /// "Disconnect": the app's keys on the identity are disabled (an IdentityUpdate signed by the
    /// MASTER key) and the app leaves the list. Answered with `Done { what: DoneWhat::Disconnect }`.
    pub(in crate::real) fn disconnect_app(&self, contract: &str) -> Result<(), CoreError> {
        let (wallet_id, keys) = self.dashpay_ready()?;
        let (chain, apps, refresh) = (self.chain.clone(), self.apps.clone(), self.refresher());
        let contract = contract.to_owned();
        self.answer_later("task:disconnect_app", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                let who = chain
                    .sign_in_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                let me = base58(&who.id_bytes());
                let app = apps
                    .get(&contract, &me)
                    .ok_or(CoreError::NotAvailable(tr!("This app is no longer in the list").into()))?;
                let login = app_connect::login_key(
                    &who.master,
                    &who.id_bytes(),
                    &contract_bytes(&app)?,
                    app.generation,
                );
                let app_keys = AppKeys::of(&login, &who.id_bytes());
                // The keys as they are now: enabled ones are disabled, nothing else.
                let (auth_data, encryption_data) =
                    (app_keys.auth_data(), app_keys.encryption_data());
                let ids: Vec<u32> = who
                    .keys
                    .iter()
                    .filter(|k| {
                        use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
                        k.disabled_at().is_none()
                            && (k.data().as_slice() == auth_data
                                || k.data().as_slice() == encryption_data)
                    })
                    .map(|k| {
                        use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
                        k.id()
                    })
                    .collect();
                if !ids.is_empty() {
                    chain
                        .disable_keys(&wallet, &who, ids)
                        .await
                        .map_err(dashpay_failure)?;
                }
                apps.remove(&contract, &me).map_err(CoreError::Failed)
            }
            .await;
            refresh();
            Event::Done {
                what: DoneWhat::Disconnect,
                result,
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_only_the_apps_of_the_shown_identity() {
        let app = |identity: &str, keys| App {
            contract: "C".into(),
            name: "@yappr".into(),
            identity: identity.into(),
            generation: 0,
            keys,
            last_used: 5,
        };
        let apps = [app("I1", Some((6, 7))), app("I2", None)];
        let shown = shown_apps(&apps, Some("I1"));
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].key, Some((6, KeyLevel::High, Vec::new())));
        assert_eq!(
            shown_apps(&apps, Some("I2"))[0].key,
            None,
            "keys not added yet"
        );
        assert!(shown_apps(&apps, None).is_empty());
    }

    /// BUG-29: «Work as» changed between the request and «Allow» — the request is out of date,
    /// not answered by the other identity; unchanged, it goes on.
    #[test]
    fn a_request_is_answered_only_as_the_identity_it_showed() {
        let (shown, other) = ([1u8; 32], [2u8; 32]);
        let sign_in = AppJob::SignIn {
            request: app_connect::KeyRequest {
                app_ephemeral: [2; 33],
                contract_id: [3; 32],
                label: "yappr".into(),
            },
            name: "@yappr".into(),
            identity: shown,
        };
        let add_keys = AppJob::AddKeys {
            app: App {
                contract: base58(&[3; 32]),
                name: "@yappr".into(),
                identity: base58(&shown),
                generation: 0,
                keys: None,
                last_used: 5,
            },
        };
        for job in [sign_in, add_keys] {
            assert!(job.still_for(&shown).is_ok());
            match job.still_for(&other) {
                Err(CoreError::NotAvailable(why)) => assert!(why.contains("out of date"), "{why}"),
                _ => panic!("answered as another identity than shown"),
            }
        }
    }
}
