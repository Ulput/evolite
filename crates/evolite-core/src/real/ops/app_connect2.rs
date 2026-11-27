//! App Connect v2 in the core's loop (FEAT-0023 round 3): a `dashpay://connect` or `dash-key:`
//! v2 link becomes a request checked against Platform and shown with the wallet's limits;
//! "Allow" with the person's choices adds the session keys and the encryption keys in
//! MASTER-signed updates, waits until they are on the identity, and only then publishes the
//! response; "Disconnect" disables the session keys (never the encryption keys) and deletes the
//! response. Nothing is written before the request has been shown and approved with the
//! password, and every rule is checked before that (`app_connect_v2::grant`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::real::sent;

use dpp::identity::IdentityPublicKey;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::identity_public_key::accessors::v1::IdentityPublicKeyGettersV1;
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
use crate::real::{AppJob, Real, dashpay, dashpay_failure, journal, loaded, now};

/// A v2 request shown and not answered yet.
pub(in crate::real) struct Connect2Job {
    pub link: v2::Link,
    /// The link as it was opened, for «Try again» (BUG-323).
    pub text: String,
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
            // default-ok: display only: a link without v shows none
            v: link_param(text, "v").unwrap_or_default(),
        },
        // TECH-mainnet-all: the transitional carrier on mainnet (DIP-0015's rule, kept), and a
        // mainnet request on another network, are two different things to say.
        Refusal::Refused(_, Part::Carrier) => AppLinkRefusal::MainnetCarrier,
        Refusal::Refused(_, Part::Network) => match link_param(text, "n") {
            // A request of the other network: said by its name (the analyst; not «(t)»).
            Some(n) if n == "m" => AppLinkRefusal::LinkForNetwork {
                network: evolite_api::Network::Mainnet,
            },
            Some(n) if n == "t" => AppLinkRefusal::LinkForNetwork {
                network: evolite_api::Network::Testnet,
            },
            // A network evolite does not know (devnet, anything else): the link is not right.
            Some(_) | None => AppLinkRefusal::Malformed,
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

/// BUG-323: a proved read the request needs failed: which one (no id in the journal, with the
/// protocol version the SDK saw), and the refusal that says it, with the link to open again.
fn read_failed(
    stage: ConnectRead,
    id: &[u8; 32],
    error: &app_chain_v2::ReadError,
    link: &str,
    protocol: Option<u32>,
) -> CoreError {
    crate::diag::record(crate::diag::Record::ConnectRead {
        stage: stage.code(),
        error: crate::diag::error_kind_of_debug(&error.debug),
        protocol,
    });
    CoreError::AppLink(AppLinkRefusal::ReadFailed {
        stage,
        id: base58(id),
        undecoded: undecoded(&error.debug),
        details: error.text.clone(),
        link: link.to_owned(),
    })
}

/// BUG-323 r1 (Should 1): Platform answered and evolite could not take the answer — a protocol,
/// decoding, version or proof error, by the error's type names; not a timeout or an unreachable
/// node. Only then «perhaps newer than this evolite».
pub(in crate::real) fn undecoded(debug: &str) -> bool {
    crate::diag::error_kind_of_debug(debug)
        .split('/')
        .any(|name| {
            ["Protocol", "Decod", "Version", "Proof"]
                .iter()
                .any(|w| name.contains(w))
        })
}

/// [`read_failed`] for what [`Chain::connect_facts`](evolite_chain::chain::Chain) says.
fn facts_failed(error: app_chain_v2::FactsError, link: &str, protocol: Option<u32>) -> CoreError {
    use app_chain_v2::FactsError;
    match error {
        FactsError::Sdk(why) => CoreError::Failed(why),
        FactsError::Group(id, e) => read_failed(ConnectRead::Group, &id, &e, link, protocol),
        FactsError::Contract(id, e) => read_failed(ConnectRead::Contract, &id, &e, link, protocol),
    }
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
    // default-ok: an optional document type: none is empty
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
        spent_before: Some(0),
    }
}

/// BUG-302: what each of `new` keys takes over from the renewed session's key of the same binding
/// — what that one had spent (`reads`, read just before the renewal) and what it had taken over
/// itself. An earlier key with no pair hands its spending to the first new key of the same target.
/// A key not read, or one that had taken over an unknown, hands over «not known»: never a 0 that
/// is not one (§8d).
pub(in crate::real) fn carry_spent(new: &mut [StoredKey], old: &App, reads: &Reads) {
    let Some(session) = &old.session else {
        return;
    };
    // What an earlier key spent, all told; `None`: not known.
    let spent_by = |was: &StoredKey| -> Option<u64> {
        let read = reads.get(&(old.contract.clone(), old.identity.clone(), was.id))?;
        let left = read.remaining?;
        was.spent_before?
            .checked_add(was.budget.saturating_sub(left))
    };
    let mut paired = vec![false; session.keys.len()];
    for key in new.iter_mut() {
        let Some(at) = session.keys.iter().position(|k| {
            k.kind == key.kind && k.target == key.target && k.document_type == key.document_type
        }) else {
            continue;
        };
        paired[at] = true;
        key.spent_before = spent_by(&session.keys[at]);
    }
    for (was, _) in session.keys.iter().zip(&paired).filter(|(_, p)| !**p) {
        if let Some(key) = new.iter_mut().find(|k| k.target == was.target) {
            key.spent_before = key
                .spent_before
                .zip(spent_by(was))
                .map(|(a, b)| a.saturating_add(b));
        }
    }
}

/// BUG-302: what is left of the budgets of `app`'s session keys, read now with its proof — what
/// was read before may be old, the app spends till the renewal. A read that fails is no read:
/// the map is left without it.
async fn fresh_reads(chain: &evolite_chain::chain::Chain, app: &App) -> Reads {
    let mut fresh = Reads::new();
    let Some(session) = &app.session else {
        return fresh;
    };
    let Ok(identity) = Identifier::from_string(&app.identity, Encoding::Base58) else {
        return fresh;
    };
    let ids = session.keys.iter().map(|k| k.id).collect();
    // A failed read: the earlier keys' spending is not known, and the renewal goes on.
    let Ok(budgets) = chain.remaining_budgets(identity, ids).await else {
        return fresh;
    };
    for (id, remaining) in budgets {
        fresh.insert(
            (app.contract.clone(), app.identity.clone(), id),
            KeyRead {
                remaining,
                disabled: false,
            },
        );
    }
    fresh
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
                spent_before: key.spent_before.map(duffs),
            }
        })
        .collect()
}

/// What app's session keys have spent of their budgets as last read, credits. BUG-350: `None`
/// unless every key of it was read — a key not read is not known, not «nothing spent» (else a
/// read without it lowers the baseline, and the next one with it counts its spending again).
pub(in crate::real) fn spent_by_reads(app: &App, reads: &Reads) -> Option<u64> {
    let session = app.session.as_ref()?;
    if session.keys.is_empty() {
        return None;
    }
    session
        .keys
        .iter()
        .map(|key| {
            reads
                .get(&(app.contract.clone(), app.identity.clone(), key.id))
                .and_then(|r| r.remaining)
                .map(|left| key.budget.saturating_sub(left))
        })
        .sum::<Option<u64>>()
}

/// BUG-350 (review nit): the keys of app's session whose budget is not read — why
/// [`spent_by_reads`] is `None` — journaled once a run per key ([`diag::Record::AppSpentUnknown`]);
/// the ones journaled by this call.
pub(in crate::real) fn note_unknown_keys(app: &App, reads: &Reads) -> Vec<u32> {
    static NOTED: std::sync::Mutex<std::collections::BTreeSet<(String, String, u32)>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    let Some(session) = &app.session else {
        return Vec::new();
    };
    let mut noted = NOTED.lock().unwrap_or_else(|e| e.into_inner());
    let mut new = Vec::new();
    for key in &session.keys {
        let at = (app.contract.clone(), app.identity.clone(), key.id);
        if reads.get(&at).is_none_or(|r| r.remaining.is_none()) && noted.insert(at) {
            crate::diag::record(crate::diag::Record::AppSpentUnknown { key: key.id });
            new.push(key.id);
        }
    }
    new
}

/// BUG-145 AC-T3: what a read of the budgets adds to History, in duffs, and the new baseline. The
/// growth of what was spent since `seen` is the line (whole duffs of the totals, so nothing is
/// lost to rounding over many reads); a fall (a renewal, a new budget) is no line, only the new
/// baseline. Not read before: everything spent so far, once.
pub(in crate::real) fn app_spent_step(seen: Option<u64>, spent: u64) -> (Duffs, u64) {
    let seen = seen.unwrap_or(0); // default-ok: never read before: everything spent so far counts
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
        limit: duffs(session.keys.iter().map(|k| k.budget).sum()),
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
) -> Result<Vec<EncStep>, ([u8; 32], app_chain_v2::ReadError)> {
    let mut steps = Vec::new();
    for binding in &request.enc {
        let current = chain
            .current_contract_key(Identifier::from(*me), binding.contract, binding.purpose)
            .await
            .map_err(|e| (binding.contract, e))?;
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
    pub(in crate::real) fn open_connect2(
        &self,
        link: v2::Link,
        text: &str,
    ) -> Result<(), CoreError> {
        let text = text.to_owned();
        let (id, keys) = self.dashpay_ready()?;
        let (chain, requests, apps, journal, pool_fees) = (
            self.chain.clone(),
            self.app_requests.clone(),
            self.apps.clone(),
            self.journal.clone(),
            self.pool_fees.clone(),
        );
        let reads = self.app_reads.clone();
        let request_id = self.next_app_request.fetch_add(1, Ordering::Relaxed) + 1;
        self.answer_later("task:open_connect2", async move {
            let result = async {
                // BUG-236: the fees every other form shows (measured by the journal), not the
                // defaults — the request's price is not below the keys screen's for the same keys.
                let fees = crate::real::snapshot::fees_from(&chain, &journal, &pool_fees, 0).await;
                let wallet = loaded(&chain, &id).await?;
                let who = chain
                    .sign_in_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                let me = who.id_bytes();
                // BUG-298: no key to sign the response with: said now, not after the password.
                if !who.can_sign_in() {
                    return Err(CoreError::AppLink(AppLinkRefusal::NoSignInKey));
                }
                let request = &link.request;
                if request.identity_hint.is_some_and(|hint| hint != me) {
                    return Err(CoreError::NotAvailable(
                        tr!("The operation is for another identity, not the one chosen in evolite")
                            .into(),
                    ));
                }
                let protocol = chain.protocol_version().await.map_err(CoreError::Failed)?;
                if protocol < MIN_PROTOCOL {
                    return Err(not_on_this_network());
                }
                // BUG-323: a read that failed says which, not «the network refused».
                let facts = chain
                    .connect_facts(request)
                    .await
                    .map_err(|e| facts_failed(e, &text, Some(protocol)))?;
                grant::check(request, &facts)
                    .map_err(|r| refusal(r, "", grant::is_system(&request.app_id)))?;
                let at_ms = now_ms();
                let offered = request
                    .auth
                    .iter()
                    .map(|b| grant::offered(b, at_ms))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|r| refusal(r, "", false))?;
                let steps = enc_steps(&chain, &keys, &me, &who.keys, request)
                    .await
                    .map_err(|(contract, e)| {
                        read_failed(ConnectRead::EncKeys, &contract, &e, &text, Some(protocol))
                    })?;
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
                    cost: evolite_api::key_plan::fee_of_keys(added, 0, &fees) + fees.document_write,
                };
                requests.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    request_id,
                    AppJob::Connect2(Box::new(Connect2Job {
                        link: link.clone(),
                        text: text.clone(),
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
        let text = job.text.clone();
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
        // BUG-208 review r1 S1: an earlier change of these keys not known to have gone through
        // may still land; a new one now would pay again. The person waits, as for a token.
        if self.unclear.keys_on_their_way(&base58(&job.identity)) {
            return Err(CoreError::NotAvailable(
                tr!("The earlier sign-in may still go through: wait two minutes, then open the app's link again").into(),
            ));
        }
        // One change of the identity's keys at a time (FEAT-0020, the review's Should 1).
        let Some(busy) = self.identity_busy.take() else {
            return Err(super::identity_keys::identity_busy());
        };
        let (chain, apps, refresh) = (self.chain.clone(), self.apps.clone(), self.refresher());
        let reads = self.app_reads.clone();
        let (journal, unclear) = (self.journal.clone(), self.unclear.clone());
        self.answer_later("task:approve_connect2", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                // BUG-145: what the update and the response cost, from the credits before and after.
                // Shared with the key update's own task (BUG-208), never copied: it holds the
                // private halves of the identity's keys.
                let who = Arc::new(
                    chain
                        .sign_in_identity(&wallet, &keys)
                        .await
                        .map_err(CoreError::NotAvailable)?,
                );
                // The identity signing in, for its line and both credits reads.
                let acting = Some(base58(&who.id_bytes()));
                let before = chain.identity_credits(&wallet, &who.id, false).await;
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
                    // A retry of the same request: the key from before is reused (DIP), only if
                    // Platform holds it to no more than this «Allow», and kept as Platform holds
                    // it; else the app sent its `appEphPub` again for other limits: refused.
                    let (id, held) = match who
                        .keys
                        .iter()
                        .find(|k| k.data() == key.data() && k.disabled_at().is_none())
                    {
                        Some(there) => match held_within(there, &key) {
                            Some(held) => (there.id(), held),
                            None => {
                                return Err(refusal(
                                    Refusal::Refused(Status::Invalid, Part::AppKey),
                                    "",
                                    false,
                                ));
                            }
                        },
                        None => {
                            next += 1;
                            add.push(NewKey {
                                key,
                                secret: secret.clone(),
                            });
                            (next - 1, *limits)
                        }
                    };
                    stored.push(stored_key(id, &b.bounds, b.level, &held));
                    auth_slots.push(secret);
                }
                // After the password too: which read failed, not «the network refused»
                // (BUG-323 r1, Should 2).
                let steps = enc_steps(&chain, &keys, &me, &who.keys, &request)
                    .await
                    .map_err(|(contract, e)| {
                        read_failed(ConnectRead::EncKeys, &contract, &e, &text, None)
                    })?;
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
                // BUG-302: the new keys keep what the renewed ones spent.
                if let (Some(connection), Some(app)) = (&request.connection_id, &previous)
                    && app.session.as_ref().is_some_and(|s| {
                        s.connection.as_deref() == Some(evolite_common::hex(connection).as_str())
                    })
                {
                    let fresh = fresh_reads(&chain, app).await;
                    {
                        // What was read just now is what the screens show too.
                        let mut kept = reads.lock().unwrap_or_else(|e| e.into_inner());
                        for (at, read) in &fresh {
                            let disabled = kept.get(at).is_some_and(|r| r.disabled);
                            kept.insert(at.clone(), KeyRead { disabled, ..*read });
                        }
                    }
                    carry_spent(&mut stored, app, &fresh);
                }
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
                        .unwrap_or_default(), // default-ok: no previous session: none to disable
                    _ => Vec::new(),
                };
                let added: Vec<IdentityPublicKey> = add.iter().map(|k| k.key.clone()).collect();
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
                let kind = journal::Kind::AppConnect {
                    app: label.clone(),
                    renewal,
                };
                // The record of this sign-in; its response is added once published.
                let record = |response: Option<Stored>| App {
                    contract: base58(&request.app_id),
                    name: job.name.clone(),
                    identity: base58(&me),
                    generation: 0,
                    keys: None,
                    last_used: now(),
                    session: Some(Session {
                        connection: request.connection_id.map(|c| evolite_common::hex(&c)),
                        keys: stored.clone(),
                        response,
                        label: request.label.clone(),
                        disconnected_at: None,
                        // The new keys' budgets are unspent (BUG-145).
                        spent_seen: Some(0),
                    }),
                };
                // BUG-208, step 1, the keys: paid on its own. A sign-in again of this request
                // whose keys are all on the identity already pays nothing here: on to the
                // response it did not publish.
                let paid_keys = !(add.is_empty() && disable.is_empty());
                let fresh = if !paid_keys {
                    None
                } else {
                    let update = {
                        let (chain, wallet, keys, who) =
                            (chain.clone(), wallet.clone(), keys.clone(), who.clone());
                        async move { chain.send_key_update(&wallet, &keys, &who, add, disable).await }
                    };
                    let shows = |again: &evolite_chain::app_chain::SignInIdentity| {
                        again.id == who.id && all_on(&added, &again.keys)
                    };
                    let outcome = sent::run(
                        update,
                        sent::SEND_WAIT,
                        |e: &String| sent::refused_before(e),
                        || async { chain.sign_in_identity(&wallet, &keys).await.ok() },
                        shows,
                    )
                    .await;
                    match outcome {
                        sent::Outcome::Done(fresh) | sent::Outcome::Shown(fresh) => Some(fresh),
                        sent::Outcome::Refused(e) => {
                            sent::refused_line(
                                &chain,
                                &wallet,
                                who.id,
                                before,
                                label,
                                base58(&me),
                                &journal,
                            )
                            .await;
                            return Err(dashpay_failure(e));
                        }
                        sent::Outcome::Unclear { late } => {
                            // Not known: its line waits for the update's late answer, which also
                            // keeps the record, so the keys show as the app's.
                            let pending = sent::Pending::new(
                                base58(&me),
                                sent::Watch::Answer,
                                Some(kind),
                                before,
                                acting,
                            );
                            let app = record(None);
                            let (chain, wallet, id) = (chain.clone(), wallet.clone(), who.id);
                            return Err(unclear.not_known(pending, late, journal, move |_, p| {
                                async move {
                                    let _ = apps.put(app);
                                    sent::line_after(chain, wallet, id, None, p).await
                                }
                            }));
                        }
                    }
                };
                // DIP: no response before every new key is on the identity.
                if let Some(fresh) = &fresh
                    && !all_on(&added, &fresh.keys)
                {
                    // Paid all the same (BUG-208): its line now.
                    let after = chain.identity_credits(&wallet, &who.id, true).await;
                    journal.add(own_line(kind.clone(), before, after, acting.clone()));
                    // BUG-243 (3): the update's own answer came, but the read after it does not
                    // show the keys yet — a node a few blocks behind. Not «not on the identity»:
                    // not known, kept, and the record kept, so the link opened again within the
                    // quiet waits (`keys_on_their_way`) and later only publishes the response.
                    let _ = apps.put(record(None));
                    return Err(unclear.not_seen(keys_not_seen(base58(&me), kind, acting)));
                }
                let fresh: &evolite_chain::app_chain::SignInIdentity = fresh.as_ref().unwrap_or(&who);
                // The earlier response: deleted by its values, best effort.
                if let Some((document, values)) = previous
                    .as_ref()
                    .and_then(|a| a.session.as_ref())
                    .and_then(|s| s.response.as_ref())
                    .and_then(stored_response)
                {
                    let _ = chain
                        .delete_connect_response(&wallet, fresh, document, &values)
                        .await;
                }
                let response = grant::response(&auth_slots, &enc_slots, &request.app_ephemeral, &me)
                    .map_err(CoreError::Failed)?;
                // BUG-208, step 2, the response.
                let published = chain.publish_connect_response(&wallet, fresh, &response).await;
                let after = chain.identity_credits(&wallet, &who.id, true).await;
                let document = match published {
                    Ok(document) => document,
                    Err(e) => {
                        let (line, keep) = after_response_failed(paid_keys);
                        // The keys were paid: their line now, and the record, so the keys show
                        // as the app's and the link opened again only publishes (step 1 skipped).
                        if line {
                            journal.add(own_line(kind, before, after, acting));
                        }
                        if keep {
                            apps.put(record(None)).map_err(CoreError::Failed)?;
                        }
                        crate::diag::platform_failed("connect2_response", &e);
                        return Err(match dashpay_failure(e) {
                            CoreError::Failed(_) if paid_keys => CoreError::NotAvailable(
                                tr!("The app's keys are on the identity, but its answer was not published: open the app's link again to finish").into(),
                            ),
                            other => other,
                        });
                    }
                };
                // Reached only when every write went through: a refused one returned above.
                journal.add(own_line(kind, before, after, acting));
                apps.put(record(Some(Stored {
                    document: base58(&document),
                    app_key_hash: evolite_common::hex(&response.app_key_hash),
                    wallet_ephemeral: evolite_common::hex(&response.wallet_ephemeral),
                    payload: evolite_common::hex(&response.payload),
                })))
                .map_err(CoreError::Failed)
            }
            .await;
            drop(busy);
            refresh();
            Event::Done {
                what: DoneWhat::App,
                result,
                ask: None,
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
                    ask: None,
                },
                Err(e) => Event::Done {
                    what: DoneWhat::AppBudgets,
                    result: Err(e),
                    ask: None,
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
        let spent = spent_by_reads(app, &reads);
        if spent.is_none() {
            note_unknown_keys(app, &reads);
        }
        spent
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
    // default-ok: no session: no session keys to disable
    let session = app.session.clone().unwrap_or_default();
    let ids = live_session_keys(&session, &who.keys);
    let who = if ids.is_empty() {
        who
    } else {
        chain
            .send_key_update(wallet, keys, &who, Vec::new(), ids)
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
/// BUG-243 (3): App Connect's keys paid and not yet seen on the identity: kept for
/// [`sent::Unclear::keys_on_their_way`] (its line is written already).
fn keys_not_seen(identity: String, kind: journal::Kind, acting: Option<String>) -> sent::Pending {
    sent::Pending::new(identity, sent::Watch::Answer, Some(kind), None, acting)
}

/// Whether every key of `added` is on the identity (`keys`), active.
fn all_on(added: &[IdentityPublicKey], keys: &[IdentityPublicKey]) -> bool {
    added.iter().all(|new| {
        keys.iter()
            .any(|k| k.data() == new.data() && k.disabled_at().is_none())
    })
}

/// The limits Platform holds `there` to, if it may do no more than `key`: `there` is the
/// identity's enabled key with the data of `key`, the session key this «Allow» would add. The
/// data depends only on `appEphPub` and the binding's index, so `there` is this request's key from
/// before (a retry: the same bounds and budget, its end worked out again at the new «Allow», so
/// only later) or an app's `appEphPub` sent again. Reused for lower limits or other bounds,
/// Platform would keep the old ones while the record showed the new: `None`.
fn held_within(there: &IdentityPublicKey, key: &IdentityPublicKey) -> Option<Limits> {
    let same = there.purpose() == key.purpose()
        && there.key_type() == key.key_type()
        && there.security_level() == key.security_level()
        && there.contract_bounds() == key.contract_bounds();
    let budget = there
        .total_budget()
        .filter(|&held| key.total_budget().is_some_and(|asked| held <= asked))?;
    let expires_at = there
        .expires_at()
        .filter(|&held| key.expires_at().is_some_and(|asked| held <= asked))?;
    same.then_some(Limits {
        budget,
        expires_at,
        changed: false,
    })
}

/// BUG-208: the response not published after the keys were paid (`paid_keys`): whether to write
/// the keys' History line now, and whether to keep the record (its keys, no response) so the
/// keys show as the app's and the link opened again only publishes. With nothing paid, neither.
fn after_response_failed(paid_keys: bool) -> (bool, bool) {
    (paid_keys, paid_keys)
}

fn callback_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| host.to_owned())
}

#[cfg(test)]
mod tests;
