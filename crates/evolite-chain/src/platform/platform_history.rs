//! Reads back from Platform what the wallet's identities did there (BUG-58, R-33): their DPNS
//! names, the contact requests they sent, their DashPay profile and their withdrawals, as
//! journal entries (`Source::Network`) for [`journal::merge`]. Every read is proved (the
//! context provider holds DPNS, DashPay and Withdrawals) and spends nothing. Contact requests
//! come from platform-wallet's DashPay state, which its own sync keeps proved: no request of
//! ours. How far withdrawals were read is kept in `platform-sync.json` ([`Marks`]); "Sync
//! again" removes that file and nothing else.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{DocumentQuery, FetchMany};
use dpp::data_contract::DataContract;
use dpp::document::{Document, DocumentV0Getters};
use dpp::identity::accessors::IdentityGettersV0;
use dpp::platform_value::Value;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use evolite_api::Timestamp;
use key_wallet_manager::WalletId;
use platform_wallet::PlatformWallet;

use crate::chain::Chain;
use crate::journal::{Entry, Reply, Source, What, WithdrawalStatus};

/// Documents per page; Platform answers at most 100.
const PAGE: u32 = 100;

/// How many identities are read at once (SYNC-5).
const AT_ONCE: usize = 4;

/// What one pass read (SYNC-5): the entries, the marks to keep once they are written, and
/// counts for the journal's `history_pass` line. An identity whose read failed keeps its marks
/// as they were, and its entries are read again by the next pass.
#[derive(Debug, Default)]
pub struct Pass {
    pub entries: Vec<Entry>,
    pub marks: Marks,
    pub identities: u32,
    pub requests: u32,
    pub failed: u32,
}

/// How far each identity was read: `{"version":1,"identities":{"<b58>":{…}}}`.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Marks {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub identities: BTreeMap<String, IdentityMarks>,
}

fn version() -> u32 {
    1
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityMarks {
    /// The `$updatedAt` (ms) of the last withdrawal read; the next read asks from it on (`>=`:
    /// two documents of the same millisecond are not lost, the key drops the repeat).
    #[serde(default)]
    pub withdrawals_updated_at: u64,
    /// The profile's `$revision` last read.
    #[serde(default)]
    pub profile_revision: u64,
    /// The withdrawals were read with their `transactionIndex`. Marks written before it was
    /// kept say no: everything is read again once, or a finished withdrawal (whose
    /// `$updatedAt` no longer moves) could never find its payout (FEAT-0005b).
    #[serde(default)]
    pub withdrawal_indexes: bool,
}

impl Marks {
    /// The file's marks; none (read everything) when it is missing or unreadable.
    pub fn load(path: &Path) -> Self {
        crate::json_file::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Marks>(&bytes).ok())
            .filter(|m| m.version == 1)
            .unwrap_or(Marks {
                version: 1,
                identities: BTreeMap::new(),
            })
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::json_file::save(path, self)
    }
}

/// One pass over the identities of `wallet`: the entries read and the marks to keep once the
/// entries are written. `names`: `@name`s known for identities (base58 → label), for whom a
/// contact request went to. `named`: the (identity, label) pairs the journal has names for — an
/// identity whose names the identity search found are all there is not asked for its names
/// again (SYNC-5: the search read the same documents).
///
/// Up to [`AT_ONCE`] identities at a time; one that fails does not stop the others (SYNC-5:
/// before, one node's failure held every identity's History back to the next pass).
pub async fn read(
    chain: &Chain,
    wallet: &PlatformWallet,
    id: WalletId,
    marks: &Marks,
    names: &std::collections::HashMap<String, Option<String>>,
    named: &std::collections::HashSet<(String, String)>,
) -> Result<Pass, String> {
    let sdk = chain.sdk().await?;
    let version = sdk.version();
    let contract = |c| {
        load_system_data_contract(c, version)
            .map(Arc::new)
            .map_err(|e| format!("{e:?}"))
    };
    let contracts = Arc::new(Contracts {
        dpns: contract(SystemDataContract::DPNS)?,
        dashpay: contract(SystemDataContract::Dashpay)?,
        withdrawals: contract(SystemDataContract::Withdrawals)?,
    });
    // The identities and their contact requests as platform-wallet holds them now.
    let mut identities = Vec::new();
    let mut entries = Vec::new();
    {
        let state = wallet.state().await;
        for m in state
            .identity_manager
            .managed_identities()
            .filter(|m| m.wallet_id == Some(id) && m.identity_index.is_some())
        {
            let me = m.identity.id();
            let b58 = me.to_string(Encoding::Base58);
            let names_known =
                names_known(&b58, m.dpns_names.iter().map(|n| n.label.as_str()), named);
            identities.push((me, names_known));
            let dashpay = m.dashpay();
            let established = dashpay.established_contacts();
            let pairs = Requests {
                sent: dashpay
                    .sent_contact_requests()
                    .iter()
                    .filter(|(them, _)| !established.contains_key(them))
                    .map(|(them, r)| (*them, r.created_at))
                    .collect(),
                incoming: dashpay
                    .incoming_contact_requests()
                    .iter()
                    .filter(|(them, _)| !established.contains_key(them))
                    .map(|(them, r)| (*them, r.created_at, m.is_sender_ignored(them)))
                    .collect(),
                both: established
                    .iter()
                    .map(|(them, c)| {
                        (
                            *them,
                            c.outgoing_request.created_at,
                            c.incoming_request.created_at,
                        )
                    })
                    .collect(),
            };
            entries.extend(pairs.entries(&me, names));
        }
    }
    let mut pass = Pass {
        entries,
        marks: marks.clone(),
        identities: identities.len() as u32,
        ..Default::default()
    };
    pass.marks.version = 1;
    let gate = Arc::new(tokio::sync::Semaphore::new(AT_ONCE));
    let mut reads = tokio::task::JoinSet::new();
    for (me, names_known) in identities {
        let b58 = me.to_string(Encoding::Base58);
        let mark = marks.identities.get(&b58).cloned().unwrap_or_default();
        let (sdk, contracts, gate) = (sdk.clone(), contracts.clone(), gate.clone());
        reads.spawn(async move {
            let _turn = gate.acquire_owned().await;
            let mut requests = 0;
            let read = read_identity(&sdk, &contracts, me, &mark, names_known, &mut requests).await;
            (b58, read, requests)
        });
    }
    while let Some(done) = reads.join_next().await {
        match done {
            Ok((b58, read, requests)) => take(&mut pass, b58, read, requests),
            // A read that panicked: its identity's marks stay.
            Err(_) => pass.failed += 1,
        }
    }
    Ok(pass)
}

/// Whether the identity `b58`'s names the search found (`labels`) are all in the journal
/// (`named`): then its names are not asked again (SYNC-5).
fn names_known<'a>(
    b58: &str,
    mut labels: impl Iterator<Item = &'a str>,
    named: &std::collections::HashSet<(String, String)>,
) -> bool {
    labels.all(|label| named.contains(&(b58.to_owned(), label.to_owned())))
}

/// One identity's read into the pass: its entries and new marks, or — failed — its marks as they
/// were, so the next pass reads it again, and the others go on (SYNC-5).
fn take(
    pass: &mut Pass,
    b58: String,
    read: Result<(Vec<Entry>, IdentityMarks), String>,
    requests: u32,
) {
    pass.requests += requests;
    match read {
        Ok((entries, mark)) => {
            pass.entries.extend(entries);
            pass.marks.identities.insert(b58, mark);
        }
        Err(e) => {
            pass.failed += 1;
            crate::diag::platform_failed("platform_history", &e);
        }
    }
}

/// The system contracts a pass reads.
struct Contracts {
    dpns: Arc<DataContract>,
    dashpay: Arc<DataContract>,
    withdrawals: Arc<DataContract>,
}

/// One identity's names (unless `names_known`), profile and withdrawals from its marks on; the
/// entries and its marks after. `requests` counts the queries sent.
async fn read_identity(
    sdk: &dash_sdk::Sdk,
    contracts: &Contracts,
    me: Identifier,
    mark: &IdentityMarks,
    names_known: bool,
    requests: &mut u32,
) -> Result<(Vec<Entry>, IdentityMarks), String> {
    let b58 = me.to_string(Encoding::Base58);
    let mut entries = Vec::new();
    let mut new = mark.clone();
    // Names: a few documents per identity, no index by time — all of them, unless the identity
    // search found only names the journal has.
    if !names_known {
        let q = query(&contracts.dpns, "domain", "records.identity", me, None)?;
        *requests += 1;
        for doc in fetch(sdk, q).await? {
            if let Some(entry) = name(&doc, &me, &b58) {
                entries.push(entry);
            }
        }
    }
    // The profile: one document, its last revision only.
    let q = query(&contracts.dashpay, "profile", "$ownerId", me, None)?;
    *requests += 1;
    if let Some(doc) = fetch(sdk, q).await?.into_iter().next() {
        let revision = doc.revision().unwrap_or(1);
        if revision != mark.profile_revision {
            entries.push(profile(&doc, &b58));
        }
        new.profile_revision = revision;
    }
    // Withdrawals: by `$updatedAt` from the mark on, page by page.
    let mut from = if mark.withdrawal_indexes {
        mark.withdrawals_updated_at
    } else {
        0
    };
    loop {
        let q = query(
            &contracts.withdrawals,
            "withdrawal",
            "$ownerId",
            me,
            Some(from),
        )?;
        *requests += 1;
        let page = fetch(sdk, q).await?;
        let full = page.len() as u32 >= PAGE;
        let last = page.iter().filter_map(|d| d.updated_at()).max();
        entries.extend(page.iter().filter_map(|d| withdrawal(d, &b58)));
        match last {
            // A full page of one millisecond would ask the same again: stop there.
            Some(last) if full && last > from => from = last,
            Some(last) => {
                from = from.max(last);
                break;
            }
            None => break,
        }
    }
    new.withdrawals_updated_at = from;
    new.withdrawal_indexes = true;
    Ok((entries, new))
}

/// `field == identity`, ordered by `$updatedAt` from `from` on when given.
fn query(
    contract: &Arc<DataContract>,
    kind: &str,
    field: &str,
    identity: Identifier,
    from: Option<u64>,
) -> Result<DocumentQuery, String> {
    let mut query = DocumentQuery::new(contract.clone(), kind)
        .map_err(|e| format!("{e:?}"))?
        .with_where(WhereClause {
            field: field.into(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(identity.to_buffer()),
        })
        .with_limit(PAGE);
    if let Some(from) = from {
        query = query
            .with_where(WhereClause {
                field: "$updatedAt".into(),
                operator: WhereOperator::GreaterThanOrEquals,
                value: Value::U64(from),
            })
            .with_order_by(OrderClause {
                field: "$updatedAt".into(),
                ascending: true,
            });
    }
    Ok(query)
}

async fn fetch(sdk: &dash_sdk::Sdk, query: DocumentQuery) -> Result<Vec<Document>, String> {
    Ok(Document::fetch_many(sdk, query)
        .await
        .map_err(|e| format!("{e:?}"))?
        .into_values()
        .flatten()
        .collect())
}

fn seconds(ms: u64) -> Timestamp {
    (ms / 1000) as Timestamp
}

fn network(at: u64, what: What, identity: &str, key: String) -> Entry {
    Entry {
        at: seconds(at),
        what,
        identity: Some(identity.to_owned()),
        key: Some(key),
        source: Source::Network,
        ..Default::default()
    }
}

/// A `domain` document: the name it registered, when the identity owns the document. Before
/// DPNS v3 `records.identity` need not be the owner, and a name passed on keeps it: a
/// registration that was someone else's is no line of ours.
fn name(doc: &Document, me: &Identifier, identity: &str) -> Option<Entry> {
    if doc.owner_id() != *me {
        return None;
    }
    let label = doc.properties().get("label")?.as_text()?.to_owned();
    let key = format!("dpns:{}", doc.id().to_string(Encoding::Base58));
    let mut entry = network(
        doc.created_at().unwrap_or(0),
        What::Name { label },
        identity,
        key,
    );
    entry.block = doc.created_at_block_height();
    // BUG-176: the document carries no fee; one registered here comes with its entry.
    entry.cost_known = false;
    Some(entry)
}

/// The `profile` document: its last save. Keyed by revision, so the next save is a line of its
/// own; its time is the revision's (`$updatedAt`), the document's first save otherwise.
fn profile(doc: &Document, identity: &str) -> Entry {
    let key = format!(
        "profile:{}:{}",
        doc.id().to_string(Encoding::Base58),
        doc.revision().unwrap_or(1)
    );
    let at = doc.updated_at().or(doc.created_at()).unwrap_or(0);
    let mut entry = network(at, What::Profile, identity, key);
    entry.block = doc
        .updated_at_block_height()
        .or(doc.created_at_block_height());
    entry.cost_known = false;
    entry
}

/// A `withdrawal` document: to which Core address, how much, and how far it got.
fn withdrawal(doc: &Document, identity: &str) -> Option<Entry> {
    use key_wallet::dashcore::{Address, ScriptBuf};
    let props = doc.properties();
    let script = props.get("outputScript")?.to_binary_bytes().ok()?;
    let to = Address::from_script(&ScriptBuf::from(script), crate::network::dash())
        .map(|a| a.to_string())
        .ok()?;
    let credits = props.get("amount")?.as_integer::<u64>()?;
    let duffs = crate::identity::credits_as_duffs(credits);
    let key = format!("wd:{}", doc.id().to_string(Encoding::Base58));
    let mut entry = network(
        doc.created_at().unwrap_or(0),
        What::Withdrawal { to },
        identity,
        key,
    );
    entry.spent = duffs;
    entry.moved = duffs;
    entry.status = props
        .get("status")
        .and_then(|s| s.as_integer::<u64>())
        .and_then(WithdrawalStatus::of);
    entry.transaction_index = props
        .get("transactionIndex")
        .and_then(|i| i.as_integer::<u64>());
    entry.block = doc.created_at_block_height();
    Some(entry)
}

/// The contact requests of one identity of ours, as platform-wallet holds them: ours only
/// (`sent`: them, ms), theirs only (`incoming`: them, ms, hidden by us) and both ways (`both`:
/// them, ours ms, theirs ms).
struct Requests {
    sent: Vec<(Identifier, u64)>,
    incoming: Vec<(Identifier, u64, bool)>,
    both: Vec<(Identifier, u64, u64)>,
}

impl Requests {
    /// One line per request and direction (BUG-58, the analyst's rules): ours to them, theirs
    /// to us, each with how it stands. Both ways, the later one answered the earlier one.
    fn entries(
        &self,
        me: &Identifier,
        names: &std::collections::HashMap<String, Option<String>>,
    ) -> Vec<Entry> {
        let mut out = Vec::new();
        for (them, at) in &self.sent {
            out.push(contact_request(me, them, *at, names));
        }
        for (them, at, hidden) in &self.incoming {
            let mut entry = request_to_us(me, them, *at, names);
            entry.reply = Some(if *hidden {
                Reply::Hidden
            } else {
                Reply::Waiting
            });
            out.push(entry);
        }
        for (them, ours, theirs) in &self.both {
            let we_answered = ours > theirs;
            let mut entry = contact_request(me, them, *ours, names);
            entry.reply = Some(if we_answered {
                Reply::Accepted
            } else {
                Reply::TheyAccepted
            });
            out.push(entry);
            let mut entry = request_to_us(me, them, *theirs, names);
            entry.reply = Some(if we_answered {
                Reply::Accepted
            } else {
                Reply::TheyAccepted
            });
            out.push(entry);
        }
        out
    }
}

/// Pairs each finished withdrawal with its payout on Core (FEAT-0005 АРХ-2, AC-T4), by the only
/// proof there is: the document's `transactionIndex` equals the asset unlock's index and the
/// output goes to the document's address (`outputScript` as the address the entry keeps; one
/// address per script). Never by amount or time. Both lines then point at each other; the
/// withdrawal says it was paid and the payout names the identity. `unlocks`: the index of each
/// asset unlock of the wallet, by txid. `identity`: an identity of the wallet as the card names
/// it. A withdrawal Platform has not finished, as far as the last proved read knows, stays
/// waiting even if a matching unlock is here.
pub fn link_payouts(
    history: &mut [evolite_api::Activity],
    entries: &[Entry],
    unlocks: &std::collections::HashMap<String, u64>,
    identity: impl Fn(&str) -> Option<evolite_api::IdentityRef>,
) {
    let finished: std::collections::HashMap<(u64, &str), &Entry> = entries
        .iter()
        .filter(|e| e.status == Some(WithdrawalStatus::Complete))
        .filter_map(|e| match &e.what {
            What::Withdrawal { to } => Some(((e.transaction_index?, to.as_str()), e)),
            _ => None,
        })
        .collect();
    let mut paid = std::collections::HashMap::new();
    for act in history.iter_mut() {
        let index = act.txid.as_deref().and_then(|t| unlocks.get(t));
        let (Some(index), Some(to)) = (index, act.address.as_deref()) else {
            continue;
        };
        let Some(entry) = finished.get(&(*index, to)) else {
            continue;
        };
        let Some(line) = entry.line.map(crate::journal::line_key) else {
            continue;
        };
        paid.insert(line.clone(), (act.key.clone(), act.block));
        let detail = act.detail.get_or_insert_default();
        detail.link = Some(line);
        detail.identity = entry.identity.as_deref().and_then(&identity);
    }
    for act in history.iter_mut() {
        if let Some((payout, block)) = paid.remove(&act.key) {
            let detail = act.detail.get_or_insert_default();
            detail.link = Some(payout);
            detail.payout = Some(evolite_api::Payout { block });
        }
    }
}

/// A contact request to us from `from`: keyed `cri:<from>:<ours>`, one line per sender however
/// often they rotated its key, at their first request's time.
fn request_to_us(
    me: &Identifier,
    from: &Identifier,
    created_at: u64,
    names: &std::collections::HashMap<String, Option<String>>,
) -> Entry {
    let from_b58 = from.to_string(Encoding::Base58);
    let me_b58 = me.to_string(Encoding::Base58);
    let mut entry = network(
        created_at,
        What::ContactRequest {
            to: shown_name(&from_b58, names),
        },
        &me_b58,
        format!("cri:{from_b58}:{me_b58}"),
    );
    entry.theirs = true;
    entry
}

/// An identity as a request names it: `@label`, else the id shortened as a contact's handle is.
fn shown_name(b58: &str, names: &std::collections::HashMap<String, Option<String>>) -> String {
    match names.get(b58).cloned().flatten() {
        Some(label) => format!("@{label}"),
        None => {
            let n = b58.chars().count();
            let head: String = b58.chars().take(6).collect();
            let tail: String = b58.chars().skip(n.saturating_sub(4)).collect();
            if n > 12 {
                format!("{head}…{tail}")
            } else {
                b58.to_owned()
            }
        }
    }
}

/// BUG-69: a contact request's line names both sides as well as the wallet knows them now, not
/// as they were named when Platform was read: the other side by `named` (an identity of ours
/// with its name, or a contact's `@name`), taken from the line's key (`cr:<ours>:<to>`,
/// `cri:<from>:<ours>`) — a request read before the name was known showed a short id for good —
/// and, when the wallet has `several` identities, the one of ours it went from or came to.
pub fn name_request_sides(
    kind: &mut evolite_api::ActivityKind,
    entry: &Entry,
    several: bool,
    named: impl Fn(&str) -> Option<String>,
) {
    use evolite_api::ActivityKind;
    let other = entry.key.as_deref().and_then(|key| {
        if let Some(rest) = key.strip_prefix("cri:") {
            rest.split(':').next()
        } else {
            key.strip_prefix("cr:")?.split(':').nth(1)
        }
    });
    let ours_named = || {
        if several {
            entry.identity.as_deref().and_then(&named)
        } else {
            None
        }
    };
    match kind {
        ActivityKind::ContactRequest { to: who, ours, .. }
        | ActivityKind::ContactRequestIn {
            from: who, ours, ..
        } => {
            if let Some(name) = other.and_then(&named) {
                *who = name;
            }
            *ours = ours_named();
        }
        _ => {}
    }
}

/// The key of our contact request to `to` sent as `me`: `cr:<ours>:<to>`. The entry written
/// when it is sent here has it at once (BUG-13), so the read of Platform finds that line by key
/// and never adds a second one, however `to` is named by then.
pub fn request_key(me: &Identifier, to: &Identifier) -> String {
    format!(
        "cr:{}:{}",
        me.to_string(Encoding::Base58),
        to.to_string(Encoding::Base58)
    )
}

/// The entry for our contact request to `@label` (`to`) sent here as `me` at `at`; `spent`: what
/// it cost in duffs. Keyed ([`request_key`]) and with its identity, as the read of Platform has
/// it: that read matches it by key and keeps its cost; a request to us from `me` (another
/// identity of ours, `cri:<me>:<to>`) stays a line of its own.
pub fn sent_request(
    me: &Identifier,
    to: &Identifier,
    label: &str,
    at: Timestamp,
    spent: evolite_api::Duffs,
) -> Entry {
    Entry {
        at,
        what: What::ContactRequest {
            to: format!("@{label}"),
        },
        spent,
        identity: Some(me.to_string(Encoding::Base58)),
        key: Some(request_key(me, to)),
        ..Default::default()
    }
}

/// The entry for our answer to the request from `from` (`who`, as the screen names them), sent
/// here as `me` at `at` for `spent` duffs (BUG-198): an answer is our contact request back, so it
/// takes that request's key ([`request_key`]); the read of Platform finds it by key and the two
/// are one line, «Contact accepted», with what it cost here.
pub fn accepted_request(
    me: &Identifier,
    from: &Identifier,
    who: &str,
    at: Timestamp,
    spent: evolite_api::Duffs,
) -> Entry {
    Entry {
        at,
        what: What::ContactRequest { to: who.to_owned() },
        spent,
        identity: Some(me.to_string(Encoding::Base58)),
        key: Some(request_key(me, from)),
        ..Default::default()
    }
}

/// A contact request we sent (or our answer to theirs), as platform-wallet holds it. Keyed by the
/// pair, ours and theirs (`cr:<ours>:<to>`), without the account reference: a rotated key is no
/// new request, and every one to them is the same line, at the first one's time (`journal::merge`).
/// Theirs to us are `cri:<from>:<ours>` ([`request_to_us`]): two identities of one wallet must
/// not meet on one key.
fn contact_request(
    me: &Identifier,
    to: &Identifier,
    created_at: u64,
    names: &std::collections::HashMap<String, Option<String>>,
) -> Entry {
    let to_b58 = to.to_string(Encoding::Base58);
    let key = request_key(me, to);
    // Named as the request written here names it.
    let shown = shown_name(&to_b58, names);
    let mut entry = network(
        created_at,
        What::ContactRequest { to: shown },
        &me.to_string(Encoding::Base58),
        key,
    );
    // BUG-176: ours, its fee unknown here; one sent from this computer comes with its entry.
    entry.cost_known = false;
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{Activity, ActivityKind, Chain, Flow, Proof};

    /// SYNC-5: an identity whose found names are all in the journal is not asked for them again;
    /// one with a name the journal lacks is.
    #[test]
    fn names_the_search_found_and_the_journal_has_are_not_asked_again() {
        let named: std::collections::HashSet<(String, String)> =
            [("ID1".to_owned(), "sam".to_owned())].into();
        assert!(names_known("ID1", ["sam"].into_iter(), &named));
        assert!(!names_known("ID1", ["sam", "new"].into_iter(), &named));
        assert!(
            !names_known("ID2", ["sam"].into_iter(), &named),
            "another identity's"
        );
        assert!(
            names_known("ID2", std::iter::empty(), &named),
            "none found: none to ask"
        );
    }

    /// SYNC-5: one identity's failure keeps its marks as they were and leaves the others' read.
    #[test]
    fn one_failure_keeps_its_marks_and_the_others_go_on() {
        let old = IdentityMarks {
            withdrawals_updated_at: 5,
            profile_revision: 2,
            withdrawal_indexes: true,
        };
        let mut pass = Pass {
            marks: Marks {
                version: 1,
                identities: [
                    ("BAD".to_owned(), old.clone()),
                    ("OK".to_owned(), old.clone()),
                ]
                .into(),
            },
            identities: 2,
            ..Default::default()
        };
        let read = IdentityMarks {
            withdrawals_updated_at: 9,
            ..old.clone()
        };
        let entry = withdrawal(1, Some(1), WithdrawalStatus::Complete, "yA");
        take(&mut pass, "BAD".into(), Err("node failed".into()), 1);
        take(&mut pass, "OK".into(), Ok((vec![entry], read.clone())), 3);
        assert_eq!(pass.failed, 1);
        assert_eq!(pass.requests, 4);
        assert_eq!(pass.entries.len(), 1);
        assert_eq!(pass.marks.identities["BAD"], old, "read again next pass");
        assert_eq!(pass.marks.identities["OK"], read);
    }

    fn withdrawal(line: u64, index: Option<u64>, status: WithdrawalStatus, to: &str) -> Entry {
        Entry {
            at: 1_000,
            what: What::Withdrawal { to: to.into() },
            spent: 500,
            moved: 500,
            identity: Some("me".into()),
            key: Some(format!("wd:{line}")),
            source: Source::Network,
            status: Some(status),
            line: Some(line),
            transaction_index: index,
            ..Default::default()
        }
    }

    fn line_of(entry: &Entry) -> Activity {
        crate::journal::activity(entry)
    }

    fn payout(txid: &str, to: &str, block: u32) -> Activity {
        Activity {
            id: 7,
            kind: ActivityKind::Received { from: None },
            chain: Chain::Core,
            at: 1_000,
            amount: 500,
            proof: Proof::ChainLock,
            txid: Some(txid.into()),
            fee: None,
            block: Some(block),
            address: Some(to.into()),
            layers: 0,
            flow: Flow::In,
            key: format!("core:{txid}"),
            note: None,
            rate: None,
            detail: None,
        }
    }

    fn named(id: &str) -> Option<evolite_api::IdentityRef> {
        Some(evolite_api::IdentityRef {
            id: id.into(),
            name: Some("kestrel".into()),
        })
    }

    fn link_of(act: &Activity) -> Option<&str> {
        act.detail.as_deref().and_then(|d| d.link.as_deref())
    }

    /// AC-T4: two withdrawals of one amount, one day, one address go to their own payouts, by
    /// the index alone; the payouts name the identity.
    #[test]
    fn equal_withdrawals_find_their_own_payouts_by_index() {
        use WithdrawalStatus::Complete;
        let entries = [
            withdrawal(1, Some(10), Complete, "yA"),
            withdrawal(2, Some(11), Complete, "yA"),
        ];
        let mut history = vec![
            line_of(&entries[0]),
            line_of(&entries[1]),
            payout("t11", "yA", 200),
            payout("t10", "yA", 100),
        ];
        let unlocks = [("t10".to_string(), 10), ("t11".to_string(), 11)].into();
        link_payouts(&mut history, &entries, &unlocks, named);
        assert_eq!(link_of(&history[0]), Some("core:t10"));
        assert_eq!(link_of(&history[1]), Some("core:t11"));
        assert_eq!(link_of(&history[3]), Some(history[0].key.as_str()));
        assert_eq!(link_of(&history[2]), Some(history[1].key.as_str()));
        let paid = history[0].detail.as_deref().and_then(|d| d.payout);
        assert_eq!(paid, Some(evolite_api::Payout { block: Some(100) }));
        let from = history[3]
            .detail
            .as_deref()
            .and_then(|d| d.identity.clone());
        assert_eq!(from.and_then(|i| i.name).as_deref(), Some("kestrel"));
    }

    /// AC-T4: an index that is another one's, an address that is not the document's, a document
    /// without an index or not finished by the last proved read: no pair, however alike.
    #[test]
    fn nothing_pairs_without_the_same_index_and_address() {
        use WithdrawalStatus::{Broadcasted, Complete};
        let cases = [
            (withdrawal(1, Some(99), Complete, "yA"), "wrong index"),
            (withdrawal(1, Some(10), Complete, "yB"), "wrong address"),
            (withdrawal(1, None, Complete, "yA"), "no index read"),
            (withdrawal(1, Some(10), Broadcasted, "yA"), "not finished"),
        ];
        for (entry, why) in cases {
            let mut history = vec![line_of(&entry), payout("t10", "yA", 100)];
            let unlocks = [("t10".to_string(), 10)].into();
            link_payouts(&mut history, std::slice::from_ref(&entry), &unlocks, named);
            assert_eq!(link_of(&history[0]), None, "{why}");
            assert_eq!(link_of(&history[1]), None, "{why}");
            assert_eq!(
                history[0].detail.as_deref().and_then(|d| d.payout),
                None,
                "{why}"
            );
        }
    }

    /// A file of marks from before the index was kept asks for every withdrawal once.
    #[test]
    fn marks_from_before_the_index_read_the_withdrawals_again() {
        let old = r#"{"version":1,"identities":{"me":{"withdrawalsUpdatedAt":77}}}"#;
        let marks: Marks = serde_json::from_str(old).expect("reads");
        let me = &marks.identities["me"];
        assert_eq!(
            (me.withdrawals_updated_at, me.withdrawal_indexes),
            (77, false)
        );
    }

    /// "Sync again" removes the marks; a missing or unreadable file reads everything again.
    #[test]
    fn marks_survive_a_restart_and_a_bad_file_is_none() {
        let dir = std::env::temp_dir().join(format!("evolite-marks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("platform-sync.json");
        assert_eq!(Marks::load(&path).identities.len(), 0);
        let mut marks = Marks::load(&path);
        marks.identities.insert(
            "me".into(),
            IdentityMarks {
                withdrawals_updated_at: 42,
                profile_revision: 3,
                withdrawal_indexes: true,
            },
        );
        marks.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["version"], 1, "{text}");
        assert_eq!(
            json["identities"]["me"]["withdrawalsUpdatedAt"], 42,
            "{text}"
        );
        assert_eq!(Marks::load(&path), marks);
        std::fs::write(&path, b"not json").unwrap();
        assert!(Marks::load(&path).identities.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_contact_request_is_named_as_written_here_and_keyed_by_the_pair() {
        let me = Identifier::from([1u8; 32]);
        let to = Identifier::from([2u8; 32]);
        let to_b58 = to.to_string(Encoding::Base58);
        let mut names = std::collections::HashMap::new();
        let entry = contact_request(&me, &to, 1_700_000_000_123, &names);
        assert_eq!(entry.at, 1_700_000_000);
        assert_eq!(entry.source, Source::Network);
        // BUG-58 r1 must 1: the pair only, no account reference.
        let me_b58 = me.to_string(Encoding::Base58);
        assert_eq!(entry.key, Some(format!("cr:{me_b58}:{to_b58}")));
        let What::ContactRequest { to: shown } = &entry.what else {
            panic!("{:?}", entry.what);
        };
        assert!(shown.contains('…'), "{shown}");
        names.insert(to_b58, Some("bob".into()));
        let entry = contact_request(&me, &to, 0, &names);
        assert_eq!(entry.what, What::ContactRequest { to: "@bob".into() });
    }

    /// BUG-58 r1 should 4: a name whose document someone else owns (registered before DPNS v3
    /// with our identity in `records.identity`, or passed on to us) is no registration of ours.
    #[test]
    fn only_a_name_the_identity_owns_is_its_registration() {
        use dpp::document::DocumentV0;
        let me = Identifier::from([1u8; 32]);
        let doc = |owner: Identifier| {
            Document::V0(DocumentV0 {
                id: Identifier::from([9u8; 32]),
                owner_id: owner,
                properties: [("label".to_owned(), Value::Text("bob".into()))].into(),
                created_at: Some(1_700_000_000_000),
                ..Default::default()
            })
        };
        let entry = name(&doc(me), &me, "me").expect("ours");
        assert_eq!(
            (entry.what, entry.at),
            (
                What::Name {
                    label: "bob".into()
                },
                1_700_000_000
            )
        );
        assert_eq!(name(&doc(Identifier::from([2u8; 32])), &me, "me"), None);
    }

    /// BUG-58 part 2 (the analyst's rules): a request to us is a line of its own, keyed apart
    /// from ours, and both ways the later request answered the earlier one.
    #[test]
    fn requests_both_ways_say_who_answered() {
        let me = Identifier::from([1u8; 32]);
        let (waiting, hidden, we, they, mine) = (
            Identifier::from([2u8; 32]),
            Identifier::from([3u8; 32]),
            Identifier::from([4u8; 32]),
            Identifier::from([5u8; 32]),
            Identifier::from([6u8; 32]),
        );
        let b58 = |id: &Identifier| id.to_string(Encoding::Base58);
        let names = std::collections::HashMap::from([(b58(&we), Some("we".to_owned()))]);
        let requests = Requests {
            sent: vec![(mine, 1_000_000)],
            incoming: vec![(waiting, 2_000_000, false), (hidden, 3_000_000, true)],
            // We answered @we's request; @they answered ours.
            both: vec![(we, 9_000_000, 8_000_000), (they, 4_000_000, 5_000_000)],
        };
        let entries = requests.entries(&me, &names);
        let find = |key: String| {
            entries
                .iter()
                .find(|e| e.key.as_deref() == Some(key.as_str()))
                .unwrap_or_else(|| panic!("{key}: {entries:?}"))
        };
        let me58 = b58(&me);
        let ours = find(format!("cr:{me58}:{}", b58(&mine)));
        assert_eq!((ours.theirs, ours.reply), (false, None));
        let w = find(format!("cri:{}:{me58}", b58(&waiting)));
        assert_eq!(
            (w.theirs, w.reply, w.at),
            (true, Some(Reply::Waiting), 2_000)
        );
        let h = find(format!("cri:{}:{me58}", b58(&hidden)));
        assert_eq!(h.reply, Some(Reply::Hidden));
        let in_we = find(format!("cri:{}:{me58}", b58(&we)));
        assert_eq!(
            (in_we.reply, &in_we.what, in_we.at),
            (
                Some(Reply::Accepted),
                &What::ContactRequest { to: "@we".into() },
                8_000
            )
        );
        let out_we = find(format!("cr:{me58}:{}", b58(&we)));
        assert_eq!(out_we.reply, Some(Reply::Accepted));
        let in_they = find(format!("cri:{}:{me58}", b58(&they)));
        assert_eq!(in_they.reply, Some(Reply::TheyAccepted));
        assert_eq!(entries.len(), 7);
        // As history lines: ours that answered theirs says so; theirs say how they stand.
        let line = |e: &Entry| crate::journal::activity(e).kind;
        assert_eq!(
            line(out_we),
            evolite_api::ActivityKind::ContactRequest {
                to: "@we".into(),
                accepting: true,
                ours: None,
            }
        );
        assert!(matches!(
            line(in_they),
            evolite_api::ActivityKind::ContactRequestIn {
                state: evolite_api::RequestState::TheyAccepted,
                ..
            }
        ));
        assert_eq!(crate::journal::activity(w).amount, 0);
    }

    /// BUG-69: a request read from Platform before a name was known (a short id for good)
    /// names both sides as the wallet knows them now; the identity of ours only when the
    /// wallet has several; a side no one has named keeps what the line had.
    #[test]
    fn a_request_names_both_sides_as_known_now() {
        use evolite_api::ActivityKind;
        let (a, b, c) = (
            Identifier::from([1u8; 32]),
            Identifier::from([2u8; 32]),
            Identifier::from([3u8; 32]),
        );
        let b58 = |id: &Identifier| id.to_string(Encoding::Base58);
        let none = std::collections::HashMap::new();
        let named = |id: &str| {
            [(b58(&a), "@evolite"), (b58(&b), "@werner555")]
                .into_iter()
                .find(|(k, _)| k == id)
                .map(|(_, v)| v.to_owned())
        };
        // Ours to our other identity, read when neither was named.
        let read = Requests {
            sent: vec![(b, 1_000_000)],
            incoming: vec![],
            both: vec![],
        }
        .entries(&a, &none);
        let line = |entry: &Entry, several: bool| {
            let mut kind = crate::journal::activity(entry).kind;
            name_request_sides(&mut kind, entry, several, named);
            kind
        };
        assert_eq!(
            line(&read[0], true),
            ActivityKind::ContactRequest {
                to: "@werner555".into(),
                accepting: false,
                ours: Some("@evolite".into()),
            }
        );
        let ActivityKind::ContactRequest { to, ours, .. } = line(&read[0], false) else {
            panic!("a request")
        };
        assert_eq!(
            (to.as_str(), ours),
            ("@werner555", None),
            "one identity: no sender"
        );
        // Theirs to us from someone not named: as it was.
        let theirs = Requests {
            sent: vec![],
            incoming: vec![(c, 2_000_000, false)],
            both: vec![],
        }
        .entries(&b, &none);
        let ActivityKind::ContactRequestIn { from, ours, .. } = line(&theirs[0], true) else {
            panic!("a request to us")
        };
        assert_eq!(from, shown_name(&b58(&c), &none));
        assert_eq!(ours.as_deref(), Some("@werner555"));
    }

    /// BUG-13 with BUG-58: a request between two identities of one wallet, sent here, is one
    /// line for the sender (its cost kept) and one for the receiver, however the read of
    /// Platform names them — then four once the other answers, never a repeat of one.
    #[test]
    fn a_request_between_our_identities_is_one_line_each_way() {
        use crate::journal::merge;
        let (a, b) = (Identifier::from([1u8; 32]), Identifier::from([2u8; 32]));
        let b58 = |id: &Identifier| id.to_string(Encoding::Base58);
        // Not named yet: the read names them by their ids, the line written here by `@b`.
        let names = std::collections::HashMap::new();
        let mut local = vec![sent_request(&a, &b, "b", 1_000, 50)];
        let read = |of_a: Requests, of_b: Requests| {
            let mut out = of_a.entries(&a, &names);
            out.extend(of_b.entries(&b, &names));
            out
        };
        let sent = || {
            read(
                Requests {
                    sent: vec![(b, 1_000_400)],
                    incoming: vec![],
                    both: vec![],
                },
                Requests {
                    sent: vec![],
                    incoming: vec![(a, 1_000_400, false)],
                    both: vec![],
                },
            )
        };
        assert!(merge(&mut local, sent()));
        assert_eq!(local.len(), 2, "{local:?}");
        let ours = &local[0];
        assert_eq!(
            (ours.key.clone(), ours.spent, ours.source, ours.theirs),
            (
                Some(format!("cr:{}:{}", b58(&a), b58(&b))),
                50,
                Source::Here,
                false
            )
        );
        assert_eq!(ours.what, What::ContactRequest { to: "@b".into() });
        let theirs = &local[1];
        assert_eq!(
            (theirs.key.clone(), theirs.theirs, theirs.reply),
            (
                Some(format!("cri:{}:{}", b58(&a), b58(&b))),
                true,
                Some(Reply::Waiting)
            )
        );
        assert!(!merge(&mut local, sent()), "read again: nothing new");
        // @b answers: its own request back, and each side's line says so.
        let answered = read(
            Requests {
                sent: vec![],
                incoming: vec![],
                both: vec![(b, 1_000_400, 2_000_000)],
            },
            Requests {
                sent: vec![],
                incoming: vec![],
                both: vec![(a, 2_000_000, 1_000_400)],
            },
        );
        assert!(merge(&mut local, answered));
        assert_eq!(local.len(), 4, "{local:?}");
        assert_eq!(
            (local[0].spent, local[0].reply),
            (50, Some(Reply::TheyAccepted))
        );
        assert_eq!(local[1].reply, Some(Reply::Accepted));
    }

    /// BUG-198 AC-T1: our answer written here and the same request read from Platform are one
    /// line, with what it cost here.
    #[test]
    fn an_answer_and_its_document_are_one_line() {
        let me = Identifier::new([1; 32]);
        let them = Identifier::new([2; 32]);
        let read = || contact_request(&me, &them, 1_700_000_003_000, &Default::default());
        // Before: the answer written without a key, as the document names them otherwise — two
        // lines, «Contact accepted» and «Contact request sent», for one answer.
        let mut before = vec![Entry {
            at: 1_700_000_000,
            what: What::ContactRequest { to: "@bob".into() },
            spent: 74_118,
            identity: Some(me.to_string(Encoding::Base58)),
            ..Default::default()
        }];
        crate::journal::merge(&mut before, vec![read()]);
        assert_eq!(before.len(), 2, "the doubled row: {before:?}");
        // Now: keyed as the read keys it — one line.
        let mut local = vec![accepted_request(&me, &them, "@bob", 1_700_000_000, 74_118)];
        crate::journal::merge(&mut local, vec![read()]);
        assert_eq!(local.len(), 1, "{local:?}");
        assert_eq!(local[0].spent, 74_118, "the cost paid here");
        assert_eq!(
            local[0].key.as_deref(),
            Some(request_key(&me, &them).as_str())
        );
    }
}
