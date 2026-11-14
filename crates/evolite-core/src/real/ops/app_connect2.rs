//! App Connect v2 in the core's loop (FEAT-0023 round 3): a `dashpay://connect` or `dash-key:`
//! v2 link becomes a request checked against Platform and shown with the wallet's limits;
//! "Allow" with the person's choices adds the session keys and the encryption keys in
//! MASTER-signed updates, waits until they are on the identity, and only then publishes the
//! response; "Disconnect" disables the session keys (never the encryption keys) and deletes the
//! response. Nothing is written before the request has been shown and approved with the
//! password, and every rule is checked before that (`app_connect_v2::grant`).

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use dpp::identity::IdentityPublicKey;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use evolite_api::*;
use evolite_chain::app_chain_v2::{self, MIN_PROTOCOL, NewKey};
use evolite_chain::app_connect_v2::grant::{self, EncStep, Limits};
use evolite_chain::app_connect_v2::keys::{self as v2keys, SLOT};
use evolite_chain::app_connect_v2::{self as v2, Bounds, EncBinding, Level, Part, Refusal, Status};
use zeroize::Zeroizing;

use super::identity::own_line;
use crate::real::connected_apps::{App, Response as Stored, Session, SessionKey as StoredKey};
use crate::real::snapshot::acting_identity;
use crate::real::{AppJob, Real, dashpay, dashpay_failure, journal, loaded, now};

/// A v2 request shown and not answered yet.
pub(in crate::real) struct Connect2Job {
    pub link: v2::Link,
    pub name: String,
    /// The identity the request was shown for (BUG-29).
    pub identity: [u8; 32],
}

/// What Platform last said of a session key ([`Command::ReadAppBudgets`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::real) struct KeyRead {
    pub remaining: Option<u64>,
    pub disabled: bool,
}

/// Session key reads by (app contract, identity, key id).
pub(in crate::real) type Reads = HashMap<(String, String, u32), KeyRead>;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn duffs(credits: u64) -> Duffs {
    (credits / grant::CREDITS_PER_DUFF) as Duffs
}

fn credits(duffs: Duffs) -> u64 {
    duffs.max(0) as u64 * grant::CREDITS_PER_DUFF
}

pub(in crate::real) fn base58(bytes: &[u8; 32]) -> String {
    app_chain_v2::base58(bytes)
}

fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != 2 * N {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn unhex_vec(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// A query parameter of the link (`v`, `n`), as the link writes it.
pub(in crate::real) fn link_param(text: &str, name: &str) -> Option<String> {
    let query = text.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.chars().take(8).collect())
    })
}

/// A refused request as the person is told (the app is told the DIP's status on its own). The
/// link's own `v` and `n` say which version or network; `system`: the app's contract is one of
/// Platform's own.
pub(in crate::real) fn refusal(refused: Refusal, text: &str, system: bool) -> CoreError {
    let why = match refused {
        Refusal::NotOurs => return CoreError::BadLink,
        Refusal::Refused(Status::Stale, _) => AppLinkRefusal::Stale,
        Refusal::Refused(_, Part::Version) => AppLinkRefusal::Version {
            v: link_param(text, "v").unwrap_or_default(),
        },
        Refusal::Refused(_, Part::Carrier) => AppLinkRefusal::Mainnet,
        Refusal::Refused(_, Part::Network) => match link_param(text, "n") {
            Some(n) if n == "m" => AppLinkRefusal::Mainnet,
            Some(n) => AppLinkRefusal::OtherNetwork { n },
            None => AppLinkRefusal::Malformed,
        },
        Refusal::Refused(Status::Unsupported, _) => AppLinkRefusal::UnknownField,
        Refusal::Refused(Status::Invalid, Part::App) if system => AppLinkRefusal::SystemContract,
        Refusal::Refused(Status::Invalid, Part::App) => AppLinkRefusal::NoContract,
        Refusal::Refused(Status::Invalid, Part::AuthBinding) => AppLinkRefusal::OthersContract,
        Refusal::Refused(Status::Invalid, Part::EncBinding) => AppLinkRefusal::NoEncryptionKeys,
        Refusal::Refused(Status::Invalid, _) => AppLinkRefusal::Malformed,
    };
    CoreError::AppLink(why)
}

fn not_on_this_network() -> CoreError {
    CoreError::AppLink(AppLinkRefusal::OldNetwork)
}

/// DashPay's `profile`, as the list and the request name it.
fn is_dashpay_profile(contract: &str, name: &str) -> bool {
    name == "profile"
        && contract
            == base58(
                &dpp::system_data_contracts::SystemDataContract::Dashpay
                    .id()
                    .to_buffer(),
            )
}

fn level(level: Level) -> KeyLevel {
    match level {
        Level::Critical => KeyLevel::Critical,
        Level::High => KeyLevel::High,
    }
}

fn binding(bounds: &Bounds) -> AppBinding {
    match bounds {
        Bounds::SingleContract(c) => AppBinding::Contract { id: base58(c) },
        Bounds::DocumentType { contract, name } if is_dashpay_profile(&base58(contract), name) => {
            AppBinding::DashPayProfile
        }
        Bounds::DocumentType { contract, name } => AppBinding::DocumentType {
            contract: base58(contract),
            name: name.clone(),
        },
        Bounds::ContractGroup(g) => AppBinding::Group { id: base58(g) },
    }
}

fn stored_binding(key: &StoredKey) -> AppBinding {
    let name = key.document_type.clone().unwrap_or_default();
    match key.kind.as_str() {
        "group" => AppBinding::Group {
            id: key.target.clone(),
        },
        "document_type" if is_dashpay_profile(&key.target, &name) => AppBinding::DashPayProfile,
        "document_type" => AppBinding::DocumentType {
            contract: key.target.clone(),
            name,
        },
        _ => AppBinding::Contract {
            id: key.target.clone(),
        },
    }
}

fn stored_key(id: u32, bounds: &Bounds, level: Level, limits: &Limits) -> StoredKey {
    let (kind, target, document_type) = match bounds {
        Bounds::SingleContract(c) => ("contract", base58(c), None),
        Bounds::DocumentType { contract, name } => {
            ("document_type", base58(contract), Some(name.clone()))
        }
        Bounds::ContractGroup(g) => ("group", base58(g), None),
    };
    StoredKey {
        id,
        critical: level == Level::Critical,
        kind: kind.into(),
        target,
        document_type,
        budget: limits.budget,
        expires_at: limits.expires_at,
    }
}

/// The session keys of a kept sign-in as the screens show them, with what Platform last said.
pub(in crate::real) fn shown_session(
    app: &App,
    reads: &Reads,
    now_ms: u64,
) -> Vec<evolite_api::SessionKey> {
    let Some(session) = &app.session else {
        return Vec::new();
    };
    session
        .keys
        .iter()
        .map(|key| {
            let read = reads.get(&(app.contract.clone(), app.identity.clone(), key.id));
            let state = if session.disconnected_at.is_some() || read.is_some_and(|r| r.disabled) {
                SessionKeyState::Disabled
            } else if key.expires_at <= now_ms {
                SessionKeyState::Expired
            } else {
                SessionKeyState::Live
            };
            evolite_api::SessionKey {
                id: key.id,
                level: if key.critical {
                    KeyLevel::Critical
                } else {
                    KeyLevel::High
                },
                binding: stored_binding(key),
                budget: duffs(key.budget),
                expires_at: (key.expires_at / 1_000) as Timestamp,
                state,
                remaining: read.and_then(|r| r.remaining).map(duffs),
            }
        })
        .collect()
}

/// What app's session keys have spent of their budgets as last read, credits; `None` while no
/// key of it was read. A key not read counts nothing.
pub(in crate::real) fn spent_by_reads(app: &App, reads: &Reads) -> Option<u64> {
    let session = app.session.as_ref()?;
    let mut any = false;
    let mut spent = 0u64;
    for key in &session.keys {
        let read = reads.get(&(app.contract.clone(), app.identity.clone(), key.id));
        if let Some(left) = read.and_then(|r| r.remaining) {
            any = true;
            spent = spent.saturating_add(key.budget.saturating_sub(left));
        }
    }
    any.then_some(spent)
}

/// BUG-145 AC-T3: what a read of the budgets adds to History, in duffs, and the new baseline. The
/// growth of what was spent since `seen` is the line (whole duffs of the totals, so nothing is
/// lost to rounding over many reads); a fall (a renewal, a new budget) is no line, only the new
/// baseline. Not read before: everything spent so far, once.
pub(in crate::real) fn app_spent_step(seen: Option<u64>, spent: u64) -> (Duffs, u64) {
    let seen = seen.unwrap_or(0);
    let line = if spent > seen {
        duffs(spent) - duffs(seen)
    } else {
        0
    };
    (line, spent)
}

/// The connection a request renews: the kept, still connected sign-in of the same `CONNECTION_ID`,
/// by its primary key (what was spent, if Platform was asked). Without a `CONNECTION_ID` a
/// sign-in is never a renewal (DIP, "Identity Updates").
pub(in crate::real) fn renewal(
    app: &App,
    connection: Option<&[u8; 16]>,
    reads: &Reads,
) -> Option<Renewal> {
    let session = app
        .session
        .as_ref()
        .filter(|s| s.disconnected_at.is_none())?;
    let wanted = evolite_common::hex(connection?);
    if session.connection.as_deref() != Some(wanted.as_str()) {
        return None;
    }
    let primary = session.keys.first()?;
    let read = reads.get(&(app.contract.clone(), app.identity.clone(), primary.id));
    Some(Renewal {
        budget: duffs(primary.budget),
        spent: read
            .and_then(|r| r.remaining)
            .map(|left| duffs(primary.budget.saturating_sub(left))),
        until: (primary.expires_at / 1_000) as Timestamp,
    })
}

/// Whether the identity already has a key of the new kind (a budget or an end): if not, old
/// clients stop reading it after this sign-in (R-6).
fn has_v1_key(keys: &[IdentityPublicKey]) -> bool {
    keys.iter().any(|k| matches!(k, IdentityPublicKey::V1(_)))
}

/// The private key of an encryption binding at `rotation`.
fn enc_secret(
    keys: &key_wallet::Wallet,
    me: &[u8; 32],
    binding: &EncBinding,
    rotation: u32,
) -> Result<Zeroizing<[u8; SLOT]>, String> {
    v2keys::derive(
        keys,
        &v2keys::encryption_key_path(
            crate::network::dash(),
            *me,
            binding.contract,
            binding.purpose,
            rotation,
        ),
    )
}

/// What a login does for each encryption binding, on the identity as it is now.
async fn enc_steps(
    chain: &evolite_chain::chain::Chain,
    keys: &key_wallet::Wallet,
    me: &[u8; 32],
    identity_keys: &[IdentityPublicKey],
    request: &v2::Request,
) -> Result<Vec<EncStep>, CoreError> {
    let mut steps = Vec::new();
    for binding in &request.enc {
        let current = chain
            .current_contract_key(Identifier::from(*me), binding.contract, binding.purpose)
            .await
            .map_err(CoreError::Failed)?;
        let ours = |rotation| {
            enc_secret(keys, me, binding, rotation)
                .ok()
                .and_then(|s| grant::compressed(&s).ok())
        };
        steps.push(grant::enc_step(
            binding,
            identity_keys,
            current.as_ref(),
            ours,
        ));
    }
    Ok(steps)
}

impl Real {
    /// Shows an App Connect v2 request: checked against Platform (the app's contract, owners,
    /// groups, the encryption keys' contracts, the protocol), with the wallet's limits.
    pub(in crate::real) fn open_connect2(&self, link: v2::Link) -> Result<(), CoreError> {
        let (id, keys) = self.dashpay_ready()?;
        let (chain, requests, apps, fees) = (
            self.chain.clone(),
            self.app_requests.clone(),
            self.apps.clone(),
            crate::real::convert::fees(0),
        );
        let reads = self.app_reads.clone();
        let request_id = self.next_app_request.fetch_add(1, Ordering::Relaxed) + 1;
        self.answer_later("task:open_connect2", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let who = chain
                    .sign_in_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                let me = who.id_bytes();
                let request = &link.request;
                if request.identity_hint.is_some_and(|hint| hint != me) {
                    return Err(CoreError::NotAvailable(
                        tr!("The operation is for another identity, not the one chosen in evolite")
                            .into(),
                    ));
                }
                if chain.protocol_version().await.map_err(CoreError::Failed)? < MIN_PROTOCOL {
                    return Err(not_on_this_network());
                }
                let facts = chain
                    .connect_facts(request)
                    .await
                    .map_err(CoreError::Failed)?;
                grant::check(request, &facts)
                    .map_err(|r| refusal(r, "", grant::is_system(&request.app_id)))?;
                let at_ms = now_ms();
                let offered = request
                    .auth
                    .iter()
                    .map(|b| grant::offered(b, at_ms))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|r| refusal(r, "", false))?;
                let steps = enc_steps(&chain, &keys, &me, &who.keys, request).await?;
                let owner = facts.contract_owner[&request.app_id];
                let owner_name = chain.name_of(Identifier::from(owner)).await.ok().flatten();
                let identity = match chain.name_of(who.id).await.ok().flatten() {
                    Some(name) => format!("@{name}"),
                    None => base58(&me),
                };
                // The brand comes from Platform; the app's own words only in quotes.
                let name = owner_name
                    .as_ref()
                    .map(|n| format!("@{n}"))
                    .unwrap_or_else(|| format!("«{}»", request.label));
                let previous = apps.get(&base58(&request.app_id), &base58(&me));
                let renews = previous.as_ref().and_then(|app| {
                    renewal(
                        app,
                        request.connection_id.as_ref(),
                        &reads.lock().unwrap_or_else(|e| e.into_inner()),
                    )
                });
                let sessions = request
                    .auth
                    .iter()
                    .zip(&offered)
                    .enumerate()
                    .map(|(i, (b, limits))| SessionOffer {
                        binding: binding(&b.bounds),
                        level: level(b.level),
                        proposed_budget: (b.total_budget != 0).then(|| duffs(b.total_budget)),
                        proposed_expiry: (b.expires_at != 0)
                            .then_some((b.expires_at / 1_000) as Timestamp),
                        budget: duffs(limits.budget),
                        expires_at: (limits.expires_at / 1_000) as Timestamp,
                        changed: limits.changed,
                        primary: i == 0,
                    })
                    .collect::<Vec<_>>();
                let encryption = request
                    .enc
                    .iter()
                    .zip(&steps)
                    .map(|(b, step)| EncryptionOffer {
                        contract: base58(&b.contract),
                        encryption: b.purpose == v2::EncPurpose::Encryption,
                        given: *step != EncStep::Omit,
                        added: *step == EncStep::Register,
                    })
                    .collect::<Vec<_>>();
                let added =
                    sessions.len() + steps.iter().filter(|s| **s == EncStep::Register).count();
                let kind = AppRequestKind::Connect2 {
                    app_id: base58(&request.app_id),
                    label: request.label.clone(),
                    owner: base58(&owner),
                    owner_name,
                    identity,
                    sessions,
                    encryption,
                    limits: SessionLimits::wallet(),
                    breaks_old_clients: !has_v1_key(&who.keys),
                    renews,
                    created_at: (request.created_at / 1_000) as Timestamp,
                    callback_host: link.callback.as_deref().and_then(callback_host),
                    cost: fees.add_key * added as Duffs + fees.document_write,
                };
                requests.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    request_id,
                    AppJob::Connect2(Box::new(Connect2Job {
                        link: link.clone(),
                        name: name.clone(),
                        identity: me,
                    })),
                );
                Ok(AppRequest {
                    id: request_id,
                    app: name,
                    kind,
                })
            }
            .await;
            Event::AppRequest(result)
        });
        Ok(())
    }

    /// "Allow" on a v2 request: the kept session keys with the chosen limits and the encryption
    /// keys go on the identity (the previous session's keys of the same connection disabled in
    /// the same update), and once all are there the response is published. Answered with
    /// `Done { what: DoneWhat::App }`.
    pub(in crate::real) fn approve_connect2(
        &self,
        job: Connect2Job,
        grant: Option<SessionGrant>,
    ) -> Result<(), CoreError> {
        let grant = grant.ok_or(CoreError::NotAvailable(
            tr!("The request is out of date: open the app's link again").into(),
        ))?;
        let request = job.link.request.clone();
        let at_ms = now_ms();
        // r3 review, Should 2: the app stops waiting after 10 minutes; a request kept open longer
        // is not answered.
        v2::fresh(request.created_at, at_ms).map_err(|r| refusal(r, "", false))?;
        let selected: Vec<bool> = grant.keys.iter().map(Option::is_some).collect();
        let kept = grant::kept(&request, &selected).ok_or(CoreError::NotAvailable(
            tr!("The app's first access cannot be left out: decline the request instead").into(),
        ))?;
        let choices: Vec<(u64, u64)> = kept
            .iter()
            .map(|&i| {
                let (budget, end) = grant.keys[i].expect("kept");
                (credits(budget), end.max(0) as u64 * 1_000)
            })
            .collect();
        let limits = grant::chosen_all(&choices, at_ms).ok_or(CoreError::NotAvailable(
            tr!("Choose a budget of at most 5 DASH and an end between an hour and a year away")
                .into(),
        ))?;
        let (wallet_id, keys) = self.dashpay_ready()?;
        let (chain, apps, refresh) = (self.chain.clone(), self.apps.clone(), self.refresher());
        let journal = self.journal.clone();
        self.answer_later("task:approve_connect2", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                // BUG-145: what the update and the response cost, from the credits before and after.
                let acting = acting_identity(&wallet).await;
                let before = chain.first_identity_credits(&wallet, false).await;
                let who = chain
                    .sign_in_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                let me = who.id_bytes();
                if me != job.identity {
                    return Err(CoreError::NotAvailable(
                        tr!("The request is out of date: open the app's link again").into(),
                    ));
                }
                if chain.protocol_version().await.map_err(CoreError::Failed)? < MIN_PROTOCOL {
                    return Err(not_on_this_network());
                }
                let net = crate::network::dash();
                let mut next = who.next_key_id();
                let mut add = Vec::new();
                let mut auth_slots = Vec::new();
                let mut stored = Vec::new();
                for (&i, limits) in kept.iter().zip(&limits) {
                    let b = &request.auth[i];
                    let path = v2keys::session_key_path(
                        net,
                        me,
                        v2keys::request_id(&request.app_ephemeral, i as u8),
                    );
                    let secret = v2keys::derive(&keys, &path).map_err(CoreError::Failed)?;
                    let key = grant::session_key(next, b, limits, &secret)
                        .map_err(CoreError::Failed)?;
                    // A retry of the same request: the key from before is reused (DIP).
                    let id = match who
                        .keys
                        .iter()
                        .find(|k| k.data() == key.data() && k.disabled_at().is_none())
                    {
                        Some(there) => there.id(),
                        None => {
                            next += 1;
                            add.push(NewKey {
                                key,
                                secret: secret.clone(),
                            });
                            next - 1
                        }
                    };
                    stored.push(stored_key(id, &b.bounds, b.level, limits));
                    auth_slots.push(secret);
                }
                let steps = enc_steps(&chain, &keys, &me, &who.keys, &request).await?;
                let mut enc_slots = Vec::new();
                for (b, step) in request.enc.iter().zip(steps) {
                    match step {
                        EncStep::Omit => {}
                        EncStep::Send(rotation) => enc_slots
                            .push(enc_secret(&keys, &me, b, rotation).map_err(CoreError::Failed)?),
                        EncStep::Register => {
                            let secret =
                                enc_secret(&keys, &me, b, 0).map_err(CoreError::Failed)?;
                            let key = grant::encryption_key(next, b, &secret)
                                .map_err(CoreError::Failed)?;
                            next += 1;
                            add.push(NewKey {
                                key,
                                secret: secret.clone(),
                            });
                            enc_slots.push(secret);
                        }
                    }
                }
                // The previous session of the same connection: its keys go in the same update.
                let previous = apps.get(&base58(&request.app_id), &base58(&me));
                let disable: Vec<u32> = match (&request.connection_id, &previous) {
                    (Some(connection), Some(app)) => app
                        .session
                        .as_ref()
                        .filter(|s| {
                            s.connection.as_deref() == Some(evolite_common::hex(connection).as_str())
                        })
                        .map(|s| {
                            s.keys
                                .iter()
                                .map(|k| k.id)
                                .filter(|id| {
                                    !stored.iter().any(|k| k.id == *id)
                                        && who
                                            .keys
                                            .iter()
                                            .any(|k| k.id() == *id && k.disabled_at().is_none())
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                let added: Vec<IdentityPublicKey> = add.iter().map(|k| k.key.clone()).collect();
                let fresh = chain
                    .update_app_keys(&wallet, &keys, &who, add, disable)
                    .await
                    .map_err(dashpay_failure)?;
                // DIP: no response before every new key is on the identity.
                let all_there = added.iter().all(|new| {
                    fresh
                        .keys
                        .iter()
                        .any(|k| k.data() == new.data() && k.disabled_at().is_none())
                });
                if !all_there {
                    return Err(CoreError::NotAvailable(
                        tr!("The keys were handed to the network, but they are not on the identity yet")
                            .into(),
                    ));
                }
                // The earlier response: deleted by its values, best effort.
                if let Some((document, values)) = previous
                    .as_ref()
                    .and_then(|a| a.session.as_ref())
                    .and_then(|s| s.response.as_ref())
                    .and_then(stored_response)
                {
                    let _ = chain
                        .delete_connect_response(&wallet, &fresh, document, &values)
                        .await;
                }
                let response = grant::response(&auth_slots, &enc_slots, &request.app_ephemeral, &me)
                    .map_err(CoreError::Failed)?;
                let document = chain
                    .publish_connect_response(&wallet, &fresh, &response)
                    .await
                    .map_err(dashpay_failure)?;
                let after = chain.first_identity_credits(&wallet, true).await;
                // The app as it calls itself (R-8), else as the sign-in showed it; its own words
                // cleaned (AC-A6).
                let label = dashpay::shown(if request.label.trim().is_empty() {
                    &job.name
                } else {
                    &request.label
                });
                // The same connection, still connected: its keys replaced (AC-A1).
                let renewal = apps
                    .get(&base58(&request.app_id), &base58(&me))
                    .and_then(|app| renewal(&app, request.connection_id.as_ref(), &Reads::new()))
                    .is_some();
                // Reached only when every write went through: a refused one returned above.
                let kind = journal::Kind::AppConnect {
                    app: label,
                    renewal,
                };
                journal.add(own_line(kind, before, after, acting));
                apps.put(App {
                    contract: base58(&request.app_id),
                    name: job.name,
                    identity: base58(&me),
                    generation: 0,
                    keys: None,
                    last_used: now(),
                    session: Some(Session {
                        connection: request.connection_id.map(|c| evolite_common::hex(&c)),
                        keys: stored,
                        response: Some(Stored {
                            document: base58(&document),
                            app_key_hash: evolite_common::hex(&response.app_key_hash),
                            wallet_ephemeral: evolite_common::hex(&response.wallet_ephemeral),
                            payload: evolite_common::hex(&response.payload),
                        }),
                        label: request.label.clone(),
                        disconnected_at: None,
                        // The new keys' budgets are unspent (BUG-145).
                        spent_seen: Some(0),
                    }),
                })
                .map_err(CoreError::Failed)
            }
            .await;
            refresh();
            Event::Done {
                what: DoneWhat::App,
                result,
            }
        });
        Ok(())
    }

    /// Reads what is left of the budgets of app `contract`'s session keys, and whether each is
    /// disabled, then shows them with a new snapshot.
    pub(in crate::real) fn read_app_budgets(&self, contract: &str) -> Result<(), CoreError> {
        let (wallet_id, keys) = self.dashpay_ready()?;
        let (chain, apps, reads, refresh) = (
            self.chain.clone(),
            self.apps.clone(),
            self.app_reads.clone(),
            self.refresher(),
        );
        let journal = self.journal.clone();
        let contract = contract.to_owned();
        self.answer_later("task:read_app_budgets", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                let who = chain
                    .sign_in_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                let me = base58(&who.id_bytes());
                let app = apps.get(&contract, &me).ok_or(CoreError::NotAvailable(
                    tr!("This app is no longer in the list").into(),
                ))?;
                read_budgets(&chain, &app, Some(&who.keys), &reads, &apps, &journal).await
            }
            .await;
            refresh();
            match result {
                Ok(()) => Event::Done {
                    what: DoneWhat::AppBudgets,
                    result: Ok(()),
                },
                Err(e) => Event::Done {
                    what: DoneWhat::AppBudgets,
                    result: Err(e),
                },
            }
        });
        Ok(())
    }
}

/// What the core needs to read the connected apps' budgets by itself (BUG-147), so a card and
/// History are right without opening «Apps».
#[derive(Clone)]
pub(in crate::real) struct AppBudgets {
    pub apps: std::sync::Arc<crate::real::connected_apps::ConnectedApps>,
    pub reads: std::sync::Arc<std::sync::Mutex<Reads>>,
    pub journal: std::sync::Arc<journal::Journal>,
}

impl AppBudgets {
    /// Reads the budgets of every app still connected; one that fails waits for the next time.
    pub async fn read_all(&self, chain: &evolite_chain::chain::Chain) {
        let connected = self.apps.all().into_iter().filter(|app| {
            app.session
                .as_ref()
                .is_some_and(|s| s.disconnected_at.is_none() && !s.keys.is_empty())
        });
        for app in connected {
            let read = read_budgets(chain, &app, None, &self.reads, &self.apps, &self.journal);
            let _ = evolite_chain::turn::timed("read_app_budgets_auto", read).await;
        }
    }
}

/// The app as History names it: as it calls itself (R-8), else as the sign-in showed it.
pub(in crate::real) fn app_label(app: &App) -> String {
    dashpay::shown(
        app.session
            .as_ref()
            .map(|s| s.label.trim())
            .filter(|l| !l.is_empty())
            .unwrap_or(&app.name),
    )
}

/// Reads what is left of the budgets of `app`'s session keys and keeps it in `reads` (BUG-147),
/// then puts what the app spent since the last read into History (BUG-145): one line an app a
/// day. `keys`: the identity's keys if read just now, for which are disabled; without them a key
/// stays as the last read found it.
pub(in crate::real) async fn read_budgets(
    chain: &evolite_chain::chain::Chain,
    app: &App,
    keys: Option<&[IdentityPublicKey]>,
    reads: &std::sync::Mutex<Reads>,
    apps: &crate::real::connected_apps::ConnectedApps,
    journal: &journal::Journal,
) -> Result<(), CoreError> {
    let Some(session) = &app.session else {
        return Ok(());
    };
    let identity = Identifier::from_string(&app.identity, Encoding::Base58)
        .map_err(|e| CoreError::Failed(e.to_string()))?;
    let ids = session.keys.iter().map(|k| k.id).collect();
    let budgets = chain
        .remaining_budgets(identity, ids)
        .await
        .map_err(CoreError::Failed)?;
    let spent = {
        let mut reads = reads.lock().unwrap_or_else(|e| e.into_inner());
        for (id, remaining) in budgets {
            let at = (app.contract.clone(), app.identity.clone(), id);
            let disabled = match keys {
                Some(keys) => keys
                    .iter()
                    .find(|k| k.id() == id)
                    .is_none_or(|k| k.disabled_at().is_some()),
                None => reads.get(&at).is_some_and(|r| r.disabled),
            };
            reads.insert(
                at,
                KeyRead {
                    remaining,
                    disabled,
                },
            );
        }
        spent_by_reads(app, &reads)
    };
    let Some(spent) = spent else {
        return Ok(());
    };
    let (line, seen) = app_spent_step(session.spent_seen, spent);
    if session.spent_seen == Some(seen) {
        return Ok(());
    }
    // The baseline first: a line written twice would count the spend twice.
    if let Some(mut kept) = apps.get(&app.contract, &app.identity) {
        if let Some(s) = kept.session.as_mut() {
            s.spent_seen = Some(seen);
        }
        apps.put(kept).map_err(CoreError::Failed)?;
    }
    if line > 0 {
        journal.add_app_spent(&app_label(app), &app.identity, line, now());
    }
    Ok(())
}

/// "Disconnect" of a v2 sign-in, as `who` (read just now): its live session keys disabled
/// (MASTER-signed), never its encryption keys; its response deleted, best effort; the app off
/// the list.
pub(in crate::real) async fn disconnect(
    chain: &evolite_chain::chain::Chain,
    wallet: &platform_wallet::PlatformWallet,
    keys: &key_wallet::Wallet,
    who: evolite_chain::app_chain::SignInIdentity,
    app: &App,
    apps: &crate::real::connected_apps::ConnectedApps,
) -> Result<(), CoreError> {
    let session = app.session.clone().unwrap_or_default();
    let ids = live_session_keys(&session, &who.keys);
    let who = if ids.is_empty() {
        who
    } else {
        chain
            .update_app_keys(wallet, keys, &who, Vec::new(), ids)
            .await
            .map_err(dashpay_failure)?
    };
    if let Some((document, values)) = session.response.as_ref().and_then(stored_response) {
        let _ = chain
            .delete_connect_response(wallet, &who, document, &values)
            .await;
    }
    // The record stays, marked, until "Remove from the list" (D2).
    apps.put(disconnected(app.clone(), now()))
        .map_err(CoreError::Failed)
}

/// `app` as "Disconnect" leaves it: marked at `at`, its response gone (deleted on Platform).
pub(in crate::real) fn disconnected(mut app: App, at: i64) -> App {
    if let Some(session) = &mut app.session {
        session.disconnected_at = Some(at);
        session.response = None;
    }
    app
}

/// "Remove from the list" (D2): only a disconnected app, and only the record on this computer.
pub(in crate::real) fn forget(
    apps: &crate::real::connected_apps::ConnectedApps,
    contract: &str,
    identity: &str,
) -> Result<(), CoreError> {
    let app = apps.get(contract, identity).ok_or(CoreError::NotAvailable(
        tr!("This app is no longer in the list").into(),
    ))?;
    if app
        .session
        .as_ref()
        .and_then(|s| s.disconnected_at)
        .is_none()
    {
        return Err(CoreError::NotAvailable(
            tr!("Disconnect the app first: its keys are still on").into(),
        ));
    }
    apps.remove(contract, identity).map_err(CoreError::Failed)
}

/// The session's keys still enabled on the identity: what "Disconnect" disables. Encryption
/// keys are not in a session, so they are never among them (DIP, "Disconnect").
pub(in crate::real) fn live_session_keys(
    session: &Session,
    identity_keys: &[IdentityPublicKey],
) -> Vec<u32> {
    session
        .keys
        .iter()
        .map(|k| k.id)
        .filter(|id| {
            identity_keys
                .iter()
                .any(|k| k.id() == *id && k.disabled_at().is_none())
        })
        .collect()
}

/// A kept response back as the delete takes it.
fn stored_response(stored: &Stored) -> Option<([u8; 32], grant::Response)> {
    let document = Identifier::from_string(
        &stored.document,
        dpp::platform_value::string_encoding::Encoding::Base58,
    )
    .ok()?
    .to_buffer();
    Some((
        document,
        grant::Response {
            app_key_hash: unhex(&stored.app_key_hash)?,
            wallet_ephemeral: unhex(&stored.wallet_ephemeral)?,
            payload: unhex_vec(&stored.payload)?,
        },
    ))
}

/// The host of the app's return link, to name it on the screen.
fn callback_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| host.to_owned())
}

#[cfg(test)]
mod tests;
