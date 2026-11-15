//! The tokens' history in History (FEAT-0019 T-1): what was received, own sends brought back
//! after a restore, and a token's balance moving with no operation of the wallet. Documents are
//! read from TokenHistory only when asked or when a balance moved (АРХ-8), turned into journal
//! entries keyed by their id, and merged; how far each was read and each token's last proved
//! balance are kept in `token-history.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use evolite_api::Timestamp;
use evolite_chain::token_history::{Balance, Cursor, Direction, Side, State, TransferDoc};
use evolite_local::journal::{self, Entry, Kind, Source};

/// What the wallet knows of a tracked token for its history.
#[derive(Clone, Debug)]
pub(in crate::real) struct TokenMeta {
    pub token_id: String,
    /// As History names it (`token_label`).
    pub label: String,
    pub decimals: u8,
    /// The token keeps its transfers in TokenHistory (`keeps_history.transfer`).
    pub keeps_history: bool,
}

/// What a new proved balance asks for (T-1.4, T-1.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::real) enum Moved {
    /// Nothing: the same balance, own sends counted in, or the first read (the point to count
    /// from, with no line).
    Nothing,
    /// It grew and the token keeps its transfers: one page of transfers to the identity.
    ReadIncoming,
    /// It fell and the token keeps its transfers: one page of transfers from the identity (a send
    /// from another computer).
    ReadOutgoing,
    /// The token keeps no transfers: a line of the change, by this much (signed units).
    Line(i128),
}

/// What a balance read now (`now`) means after the last proved one (`last`), the own sends of
/// the token since then (`sent_since`, units) counted back in.
pub(in crate::real) fn moved(
    last: Option<Balance>,
    now: u64,
    sent_since: u64,
    keeps_history: bool,
) -> Moved {
    let Some(last) = last else {
        return Moved::Nothing;
    };
    let change = i128::from(now) + i128::from(sent_since) - i128::from(last.units);
    match (change.signum(), keeps_history) {
        (0, _) => Moved::Nothing,
        (1, true) => Moved::ReadIncoming,
        (_, true) => Moved::ReadOutgoing,
        (_, false) => Moved::Line(change),
    }
}

/// The units of `token` (its label) `identity` sent from this computer after `since`.
pub(in crate::real) fn sent_since(
    entries: &[Entry],
    identity: &str,
    label: &str,
    since: Timestamp,
) -> u64 {
    entries
        .iter()
        .filter(|e| e.identity.as_deref() == Some(identity) && e.at > since)
        .filter_map(|e| match &e.kind {
            Some(Kind::TokenSent {
                token,
                units: Some(units),
                ..
            }) if token == label => Some(*units),
            _ => None,
        })
        .sum()
}

/// A `transfer` document as a journal entry of `identity` (T-1.2, T-1.3): received when it came
/// to the identity, else its own send (a line only after a restore; otherwise it keys the send).
pub(in crate::real) fn entry_of_doc(doc: &TransferDoc, identity: &str, meta: &TokenMeta) -> Entry {
    let amount = evolite_api::tokens::amount_text(doc.amount, meta.decimals);
    let kind = if doc.to == identity {
        Kind::TokenReceived {
            token: meta.label.clone(),
            from: doc.from.clone(),
            amount,
            units: doc.amount,
        }
    } else {
        Kind::TokenSent {
            token: meta.label.clone(),
            to: doc.to.clone(),
            amount,
            units: Some(doc.amount),
            to_id: Some(doc.to.clone()),
        }
    };
    let mut entry = journal::entry_of(kind, doc.at(), 0, Some(identity.to_owned()));
    entry.key = Some(format!("tokx:{}", doc.id));
    entry.source = Source::Network;
    entry
}

/// A `claim` document of TokenHistory as a journal entry (FEAT-0019 T-2.7): what `identity`
/// claimed, keyed by the document, so a claim made here and its document are one line, and one
/// made elsewhere or before a restore comes back (R-19). Claims are always kept by Platform.
pub(in crate::real) fn entry_of_claim(
    doc: &evolite_chain::chain::ClaimDoc,
    identity: &str,
    meta: &TokenMeta,
) -> Entry {
    let kind = Kind::TokenClaimed {
        token: meta.label.clone(),
        amount: evolite_api::tokens::amount_text(doc.amount, meta.decimals),
        units: doc.amount,
    };
    let at = (doc.at_ms / 1_000) as Timestamp;
    let mut entry = journal::entry_of(kind, at, 0, Some(identity.to_owned()));
    entry.key = Some(format!("tokx:{}", doc.id));
    entry.source = Source::Network;
    entry
}

/// A balance's change with no operation (UC-5в) as a journal entry, timed at the read.
pub(in crate::real) fn entry_of_change(
    identity: &str,
    meta: &TokenMeta,
    change: i128,
    at: Timestamp,
) -> Entry {
    let units = change.unsigned_abs().min(u128::from(u64::MAX)) as u64;
    let kind = Kind::TokenBalance {
        token: meta.label.clone(),
        amount: evolite_api::tokens::amount_text(units, meta.decimals),
        units,
        up: change > 0,
    };
    let mut entry = journal::entry_of(kind, at, 0, Some(identity.to_owned()));
    entry.key = Some(format!("tokbal:{identity}:{}:{at}", meta.token_id));
    entry.source = Source::Network;
    entry
}

/// The states of files that did not read, kept for this run (review r1, should 2).
static UNREADABLE: Mutex<BTreeMap<PathBuf, State>> = Mutex::new(BTreeMap::new());

/// The state, and whether its file may be written. A file that does not read is left as it is
/// and the history goes on from a state in memory for this run, as `lock-status.json` does
/// (review r1, should 2): balances are noted from the first read, pages start from the newest.
pub(in crate::real) fn load(path: &Path) -> (State, bool) {
    let mut kept = UNREADABLE.lock().unwrap_or_else(|e| e.into_inner());
    match State::load(path) {
        Ok(state) => {
            kept.remove(path);
            (state, true)
        }
        Err(_) => {
            if !kept.contains_key(path) {
                // One line, no path (T-1.8).
                crate::diag::platform_failed("token_history_file", "unreadable");
            }
            (kept.entry(path.to_owned()).or_default().clone(), false)
        }
    }
}

/// Keeps `state`: in its file when [`load`] said it may be written, in memory when not.
fn keep(path: &Path, state: State, writable: bool) {
    if writable {
        if let Err(e) = state.save(path) {
            crate::diag::platform_failed("token_history", &e);
        }
    } else {
        UNREADABLE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path.to_owned(), state);
    }
}

/// A page to read after a balance moved, and the balance to note once it was read: noted before,
/// a page that failed would never be asked again (review r1, should 1).
pub(in crate::real) type Page = (TokenMeta, Side, Balance);

/// After a proved read of `balances` (token id → units) of `identity`: notes each, and asks for
/// what the change means (T-1.4, T-1.5). Returns the pages to read and the lines to add; the
/// state is updated in place, except the balances of the pages, noted when their page is read.
pub(in crate::real) fn after_balances(
    state: &mut State,
    entries: &[Entry],
    identity: &str,
    tokens: &[TokenMeta],
    balances: &[(String, u64)],
    now: Timestamp,
) -> (Vec<Page>, Vec<Entry>) {
    let mut pages = Vec::new();
    let mut lines = Vec::new();
    for (token_id, units) in balances {
        let Some(meta) = tokens.iter().find(|t| &t.token_id == token_id) else {
            continue;
        };
        let last = state.balance(identity, token_id);
        let since = last.map_or(0, |b| b.at);
        let sent = sent_since(entries, identity, &meta.label, since);
        let balance = Balance {
            units: *units,
            at: now,
        };
        match moved(last, *units, sent, meta.keeps_history) {
            Moved::Nothing => {}
            Moved::ReadIncoming => {
                pages.push((meta.clone(), Side::To, balance));
                continue;
            }
            Moved::ReadOutgoing => {
                pages.push((meta.clone(), Side::From, balance));
                continue;
            }
            Moved::Line(change) => lines.push(entry_of_change(identity, meta, change, now)),
        }
        state.set_balance(identity, token_id, balance);
    }
    (pages, lines)
}

/// Where the pages come from: the chain, or a stand-in in tests.
pub(in crate::real) trait Pages {
    async fn page(
        &self,
        token: &str,
        identity: &str,
        side: Side,
        dir: Direction,
        cursor: Option<&Cursor>,
    ) -> Result<(Vec<TransferDoc>, Cursor), String>;
}

impl Pages for evolite_chain::chain::Chain {
    async fn page(
        &self,
        token: &str,
        identity: &str,
        side: Side,
        dir: Direction,
        cursor: Option<&Cursor>,
    ) -> Result<(Vec<TransferDoc>, Cursor), String> {
        self.token_transfers(token, identity, side, dir, cursor)
            .await
    }
}

/// Notes how far a page read: its own direction's cursor, and, for the first page read, the
/// other direction's too (the newest page is where «newer» starts and «older» goes on from).
pub(in crate::real) fn note_page(
    state: &mut State,
    identity: &str,
    token: &str,
    side: Side,
    dir: Direction,
    docs: &[TransferDoc],
    next: Cursor,
) {
    let first = state
        .cursor(identity, token, side, Direction::Newer)
        .is_none()
        && state
            .cursor(identity, token, side, Direction::Older)
            .is_none();
    if first && !docs.is_empty() {
        // The newest document read is where newer ones start; the oldest, where older go on.
        let newest = docs.iter().max_by_key(|d| d.at_ms).expect("not empty");
        let oldest = docs.iter().min_by_key(|d| d.at_ms).expect("not empty");
        let at = |ms: u64| Cursor {
            at_ms: ms,
            ids: docs
                .iter()
                .filter(|d| d.at_ms == ms)
                .map(|d| d.id.clone())
                .collect(),
        };
        state.set_cursor(identity, token, side, Direction::Newer, at(newest.at_ms));
        state.set_cursor(identity, token, side, Direction::Older, at(oldest.at_ms));
        return;
    }
    if !docs.is_empty() {
        state.set_cursor(identity, token, side, dir, next);
    }
}

/// Now, in seconds.
fn now() -> Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as Timestamp)
}

/// One page of `meta`'s transfers of `side` for `identity` in `dir`, merged into the journal and
/// noted in `state`; how many documents it had. With no cursor yet, the first page is the
/// newest one, whichever way was asked.
async fn read_page(
    pages: &impl Pages,
    journal: &journal::Journal,
    state: &mut State,
    identity: &str,
    meta: &TokenMeta,
    side: Side,
    dir: Direction,
) -> Result<usize, String> {
    // For the diag record: newer pages follow a balance that moved, older ones are asked for.
    let why = match dir {
        Direction::Newer => "moved",
        Direction::Older => "asked",
    };
    let cursor = state.cursor(identity, &meta.token_id, side, dir).cloned();
    let dir = if cursor.is_none() {
        Direction::Older
    } else {
        dir
    };
    let (docs, next) = pages
        .page(&meta.token_id, identity, side, dir, cursor.as_ref())
        .await?;
    let entries = docs
        .iter()
        .map(|d| entry_of_doc(d, identity, meta))
        .collect();
    journal.merge_token_network(entries)?;
    note_page(state, identity, &meta.token_id, side, dir, &docs, next);
    // Only counts in the journal (T-1.8): no token, identity, id or amount.
    crate::diag::record(crate::diag::Record::TokenHistory {
        side: match side {
            Side::To => "to",
            Side::From => "from",
        },
        why,
        docs: docs.len() as u32,
    });
    Ok(docs.len())
}

/// After a proved read of `identity`'s token balances (FEAT-0019 T-1.4, T-1.5): a token without
/// history that moved gets its line; one with history that moved, a page of its transfers that
/// way (and nothing is asked of a balance that did not move).
pub(in crate::real) async fn after_facts(
    chain: std::sync::Arc<evolite_chain::chain::Chain>,
    journal: std::sync::Arc<journal::Journal>,
    path: PathBuf,
    identity: String,
    metas: Vec<TokenMeta>,
    balances: Vec<(String, u64)>,
) {
    after_facts_from(&*chain, &journal, &path, &identity, &metas, &balances).await;
}

/// [`after_facts`] with its pages from `pages`.
async fn after_facts_from(
    pages: &impl Pages,
    journal: &journal::Journal,
    path: &Path,
    identity: &str,
    metas: &[TokenMeta],
    balances: &[(String, u64)],
) {
    let (mut state, writable) = load(path);
    let entries = journal.entries();
    let (to_read, lines) = after_balances(&mut state, &entries, identity, metas, balances, now());
    if !lines.is_empty()
        && let Err(e) = journal.merge_network(lines)
    {
        crate::diag::platform_failed("token_history", &e);
    }
    for (meta, side, balance) in to_read {
        match read_page(
            pages,
            journal,
            &mut state,
            identity,
            &meta,
            side,
            Direction::Newer,
        )
        .await
        {
            // The balance is noted once its page is read; until then the next read of the
            // balances sees it moved and asks again (review r1, should 1).
            Ok(_) => state.set_balance(identity, &meta.token_id, balance),
            Err(e) => crate::diag::platform_failed("token_history", &e),
        }
    }
    keep(path, state, writable);
}

/// History's «Tokens» opened, a token chosen in it, or «Show earlier» (`earlier`): one page of
/// the transfers of `metas` (only those that keep a history) to and from `identity`, older than
/// what was read — the newest first when none was. Nothing is read in the background.
pub(in crate::real) async fn read_asked(
    chain: std::sync::Arc<evolite_chain::chain::Chain>,
    journal: std::sync::Arc<journal::Journal>,
    path: PathBuf,
    identity: String,
    metas: Vec<TokenMeta>,
    earlier: bool,
) {
    let (mut state, writable) = load(&path);
    // T-2.7: the claims, once each time the layer opens (not for «Show earlier»).
    if !earlier {
        for meta in &metas {
            match chain.token_claims(&meta.token_id, &identity).await {
                Ok(docs) => {
                    let lines = docs
                        .iter()
                        .map(|d| entry_of_claim(d, &identity, meta))
                        .collect();
                    if let Err(e) = journal.merge_token_network(lines) {
                        crate::diag::platform_failed("token_history", &e);
                    }
                    // Only counts (T-1.8).
                    crate::diag::record(crate::diag::Record::TokenHistory {
                        side: "claim",
                        why: "asked",
                        docs: docs.len() as u32,
                    });
                }
                Err(e) => crate::diag::platform_failed("token_history", &e),
            }
        }
    }
    for meta in metas.iter().filter(|m| m.keeps_history) {
        for side in [Side::To, Side::From] {
            // Opened again: only what was never read; «Show earlier»: the next older page.
            let read = state
                .cursor(&identity, &meta.token_id, side, Direction::Older)
                .is_some();
            if read && !earlier {
                continue;
            }
            if let Err(e) = read_page(
                &*chain,
                &journal,
                &mut state,
                &identity,
                meta,
                side,
                Direction::Older,
            )
            .await
            {
                crate::diag::platform_failed("token_history", &e);
            }
        }
    }
    keep(&path, state, writable);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(keeps_history: bool) -> TokenMeta {
        TokenMeta {
            token_id: "TOK".into(),
            label: "evoliteQA".into(),
            decimals: 2,
            keeps_history,
        }
    }

    fn sent(at: Timestamp, units: u64) -> Entry {
        journal::entry_of(
            Kind::TokenSent {
                token: "evoliteQA".into(),
                to: "@bob".into(),
                amount: "x".into(),
                units: Some(units),
                to_id: Some("BOB".into()),
            },
            at,
            1_000,
            Some("ME".into()),
        )
    }

    /// T-1.4: a token without history — growth and fall are lines; an own send between two reads
    /// is no line; the first read is the point to count from, with no line.
    #[test]
    fn a_balance_without_history_moves_into_lines() {
        let last = Some(Balance {
            units: 1_000,
            at: 10,
        });
        assert_eq!(
            moved(None, 1_000, 0, false),
            Moved::Nothing,
            "the first read"
        );
        assert_eq!(moved(last, 1_000, 0, false), Moved::Nothing);
        assert_eq!(moved(last, 1_500, 0, false), Moved::Line(500));
        assert_eq!(moved(last, 700, 0, false), Moved::Line(-300));
        assert_eq!(moved(last, 700, 300, false), Moved::Nothing, "the own send");
        assert_eq!(
            moved(last, 900, 300, false),
            Moved::Line(200),
            "and more came"
        );
    }

    /// T-1.5: a token with history — a page of transfers only when the balance moved beyond the
    /// own sends; an unchanged balance asks nothing.
    #[test]
    fn a_page_is_read_only_when_the_balance_moved() {
        let last = Some(Balance {
            units: 1_000,
            at: 10,
        });
        assert_eq!(moved(last, 1_000, 0, true), Moved::Nothing, "no request");
        assert_eq!(
            moved(last, 700, 300, true),
            Moved::Nothing,
            "own send: no request"
        );
        assert_eq!(moved(last, 1_200, 0, true), Moved::ReadIncoming);
        assert_eq!(moved(last, 800, 0, true), Moved::ReadOutgoing);
    }

    /// The own sends counted are this identity's, of this token, after the last read.
    #[test]
    fn own_sends_since_the_last_read_are_counted() {
        let mut other = sent(50, 7);
        other.identity = Some("OTHER".into());
        let entries = vec![sent(5, 1), sent(20, 300), other];
        assert_eq!(sent_since(&entries, "ME", "evoliteQA", 10), 300);
        assert_eq!(sent_since(&entries, "ME", "another", 10), 0);
    }

    /// T-1.4, T-1.5 together: the first reads only note; later, a token without history gets a
    /// line, one with history a page, and the balances are kept.
    #[test]
    fn balances_after_a_read() {
        let mut state = State::default();
        let tokens = vec![
            meta(true),
            TokenMeta {
                token_id: "PLAIN".into(),
                label: "plain".into(),
                ..meta(false)
            },
        ];
        let first = vec![("TOK".to_owned(), 100), ("PLAIN".to_owned(), 50)];
        let (pages, lines) = after_balances(&mut state, &[], "ME", &tokens, &first, 10);
        assert!(
            pages.is_empty() && lines.is_empty(),
            "the first read only notes"
        );
        let same = first.clone();
        let (pages, lines) = after_balances(&mut state, &[], "ME", &tokens, &same, 20);
        assert!(
            pages.is_empty() && lines.is_empty(),
            "nothing moved: nothing asked"
        );
        let grew = vec![("TOK".to_owned(), 150), ("PLAIN".to_owned(), 80)];
        let (pages, lines) = after_balances(&mut state, &[], "ME", &tokens, &grew, 30);
        assert_eq!(pages.len(), 1);
        assert_eq!(
            (pages[0].0.token_id.as_str(), pages[0].1),
            ("TOK", Side::To)
        );
        assert_eq!(
            state.balance("ME", "TOK"),
            Some(Balance { units: 100, at: 20 }),
            "noted when its page is read"
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].kind,
            Some(Kind::TokenBalance {
                token: "plain".into(),
                amount: evolite_api::tokens::amount_text(30, 2),
                units: 30,
                up: true,
            })
        );
        assert_eq!(
            state.balance("ME", "PLAIN"),
            Some(Balance { units: 80, at: 30 })
        );
    }

    /// A stand-in Platform: one document from Alice to ME, or a failure (a node away).
    struct Stub {
        fail: bool,
    }

    impl Pages for Stub {
        async fn page(
            &self,
            _token: &str,
            _identity: &str,
            side: Side,
            _dir: Direction,
            _cursor: Option<&Cursor>,
        ) -> Result<(Vec<TransferDoc>, Cursor), String> {
            if self.fail {
                return Err("unavailable".into());
            }
            let docs = match side {
                Side::To => vec![TransferDoc {
                    id: "D1".into(),
                    from: "ALICE".into(),
                    to: "ME".into(),
                    amount: 50,
                    at_ms: 5_000,
                }],
                Side::From => vec![],
            };
            Ok((docs, Cursor::default()))
        }
    }

    /// A state file and a journal in a folder of the test's own.
    fn folder(name: &str) -> (PathBuf, journal::Journal) {
        let dir = std::env::temp_dir().join(format!(
            "evolite-token-history-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (
            dir.join("token-history.json"),
            journal::Journal::new(dir.join("history.json")),
        )
    }

    fn received(journal: &journal::Journal) -> usize {
        journal
            .entries()
            .iter()
            .filter(|e| matches!(e.kind, Some(Kind::TokenReceived { .. })))
            .count()
    }

    fn tok(units: u64) -> [(String, u64); 1] {
        [("TOK".to_owned(), units)]
    }

    /// Review r1, should 1: a page that failed leaves the last balance as it was, so the next
    /// read of the balances sees it moved again and asks again; the transfer is not lost.
    #[tokio::test]
    async fn a_page_that_failed_is_asked_again() {
        let (path, journal) = folder("failed");
        let metas = [meta(true)];
        let (ok, down) = (Stub { fail: false }, Stub { fail: true });
        after_facts_from(&ok, &journal, &path, "ME", &metas, &tok(100)).await;
        after_facts_from(&down, &journal, &path, "ME", &metas, &tok(150)).await;
        let units = |path: &Path| {
            State::load(path)
                .unwrap()
                .balance("ME", "TOK")
                .map(|b| b.units)
        };
        assert_eq!(
            units(&path),
            Some(100),
            "the page failed: the balance waits"
        );
        assert_eq!(received(&journal), 0);
        after_facts_from(&ok, &journal, &path, "ME", &metas, &tok(150)).await;
        assert_eq!(received(&journal), 1, "asked again, and the line came");
        assert_eq!(units(&path), Some(150));
    }

    /// Review r1, should 2: a state file that does not read turns nothing off. It is left byte
    /// for byte, and the history goes on from a state in memory.
    #[tokio::test]
    async fn a_broken_file_is_left_and_history_goes_on() {
        let (path, journal) = folder("broken");
        std::fs::write(&path, b"{ not json").unwrap();
        let metas = [meta(true)];
        let ok = Stub { fail: false };
        after_facts_from(&ok, &journal, &path, "ME", &metas, &tok(100)).await;
        after_facts_from(&ok, &journal, &path, "ME", &metas, &tok(150)).await;
        assert_eq!(
            received(&journal),
            1,
            "the balance grew: the transfer is a line"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }

    /// T-2.7: a claim document is a «Received …: distribution» line, keyed by its id, in the
    /// token's decimals, timed at the document.
    #[test]
    fn a_claim_document_is_a_distribution_line() {
        let doc = evolite_chain::chain::ClaimDoc {
            id: "C1".into(),
            amount: 2_400,
            kind: 1,
            at_ms: 5_000_000,
        };
        let entry = entry_of_claim(&doc, "ME", &meta(true));
        assert_eq!(entry.key.as_deref(), Some("tokx:C1"));
        assert_eq!(entry.at, 5_000);
        assert_eq!(entry.source, Source::Network);
        assert_eq!(
            entry.kind,
            Some(Kind::TokenClaimed {
                token: "evoliteQA".into(),
                amount: "24.00".into(),
                units: 2_400,
            })
        );
    }

    /// A document to the identity is received; one from it is its own send, keyed by the id.
    #[test]
    fn a_document_is_received_or_sent() {
        let doc = |from: &str, to: &str| TransferDoc {
            id: "D".into(),
            from: from.into(),
            to: to.into(),
            amount: 1_234,
            at_ms: 5_000,
        };
        let received = entry_of_doc(&doc("ALICE", "ME"), "ME", &meta(true));
        assert!(matches!(
            received.kind,
            Some(Kind::TokenReceived { units: 1_234, .. })
        ));
        assert_eq!((received.key.as_deref(), received.at), (Some("tokx:D"), 5));
        let own = entry_of_doc(&doc("ME", "BOB"), "ME", &meta(true));
        assert!(matches!(
            own.kind,
            Some(Kind::TokenSent {
                units: Some(1_234),
                ..
            })
        ));
    }
}
