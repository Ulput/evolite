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

use super::chain::Chain;
use super::journal::{Entry, Reply, Source, What, WithdrawalStatus};

/// Documents per page; Platform answers at most 100.
const PAGE: u32 = 100;

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
}

impl Marks {
    /// The file's marks; none (read everything) when it is missing or unreadable.
    pub fn load(path: &Path) -> Self {
        super::json_file::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Marks>(&bytes).ok())
            .filter(|m| m.version == 1)
            .unwrap_or(Marks {
                version: 1,
                identities: BTreeMap::new(),
            })
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        super::json_file::save(path, self)
    }
}

/// One pass over the identities of `wallet`: the entries read and the marks to keep once the
/// entries are written. `names`: `@name`s known for identities (base58 → label), for whom a
/// contact request went to.
pub async fn read(
    chain: &Chain,
    wallet: &PlatformWallet,
    id: WalletId,
    marks: &Marks,
    names: &std::collections::HashMap<String, Option<String>>,
) -> Result<(Vec<Entry>, Marks), String> {
    let sdk = chain.sdk().await?;
    let version = sdk.version();
    let contract = |c| {
        load_system_data_contract(c, version)
            .map(Arc::new)
            .map_err(|e| format!("{e:?}"))
    };
    let (dpns, dashpay, withdrawals) = (
        contract(SystemDataContract::DPNS)?,
        contract(SystemDataContract::Dashpay)?,
        contract(SystemDataContract::Withdrawals)?,
    );
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
            identities.push(me);
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
    let mut next = marks.clone();
    next.version = 1;
    for me in identities {
        let b58 = me.to_string(Encoding::Base58);
        let mark = marks.identities.get(&b58).cloned().unwrap_or_default();
        let new = next.identities.entry(b58.clone()).or_default();
        // Names: a few documents per identity, no index by time — all of them each pass.
        let q = query(&dpns, "domain", "records.identity", me, None)?;
        for doc in fetch(&sdk, q).await? {
            if let Some(entry) = name(&doc, &me, &b58) {
                entries.push(entry);
            }
        }
        // The profile: one document, its last revision only.
        let q = query(&dashpay, "profile", "$ownerId", me, None)?;
        if let Some(doc) = fetch(&sdk, q).await?.into_iter().next() {
            let revision = doc.revision().unwrap_or(1);
            if revision != mark.profile_revision {
                entries.push(profile(&doc, &b58));
            }
            new.profile_revision = revision;
        }
        // Withdrawals: by `$updatedAt` from the mark on, page by page.
        let mut from = mark.withdrawals_updated_at;
        loop {
            let q = query(&withdrawals, "withdrawal", "$ownerId", me, Some(from))?;
            let page = fetch(&sdk, q).await?;
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
    }
    Ok((entries, next))
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
    let duffs = super::identity::credits_as_duffs(credits);
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
    network(
        created_at,
        What::ContactRequest { to: shown },
        &me.to_string(Encoding::Base58),
        key,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let line = |e: &Entry| super::super::journal::activity(e).kind;
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
        assert_eq!(super::super::journal::activity(w).amount, 0);
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
            let mut kind = super::super::journal::activity(entry).kind;
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
        use super::super::journal::merge;
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
}
