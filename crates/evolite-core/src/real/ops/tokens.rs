//! Platform tokens (FEAT-0019 part 3): "Add token" by id, following a token in `tokens.json`.
//! Testnet only (R-9): on mainnet every token command is answered at once and nothing is asked
//! of the network (AC-T6).

use std::path::Path;
use std::sync::Mutex;

use evolite_api::tokens::TokenInfo;
use evolite_local::wallet_tokens::{Origin, Tracked, WalletTokens};

use super::names::read_within;
use crate::real::*;

/// How long one search for a token may take.
const FIND_WAIT: Duration = Duration::from_secs(60);

/// How often the shown identity's balances, pauses and freezes are read again while the wallet
/// is open: once a minute, as platform-wallet's own token sync (АРХ-1).
const FACTS_EVERY: Duration = Duration::from_secs(60);

/// How long one read of those facts may take.
const FACTS_WAIT: Duration = Duration::from_secs(60);

/// How "Add token" looks for tokens.
#[derive(Clone, Copy)]
enum Search {
    Id,
    Keyword,
}

/// What the searches found, kept until the person adds a token from them.
#[derive(Default)]
struct Seen {
    /// By token id.
    tokens: HashMap<String, TokenInfo>,
    /// Proved contracts by contract id, as `tokens.json` keeps them.
    contracts: HashMap<String, Vec<u8>>,
}

pub(in crate::real) struct TokenState {
    seen: Arc<Mutex<Seen>>,
    /// `tokens.json`; `None` on mainnet, where it is never opened (R-9).
    pub(in crate::real) file: Option<Arc<WalletTokens>>,
    /// The last proved facts: of which identity, when read, and what.
    facts: Arc<Mutex<Option<(String, std::time::Instant, chain::TokenFacts)>>>,
    /// A refresh of the facts is under way.
    reading: Arc<AtomicBool>,
    /// The kept contracts went to the provider (once per run, AC-T2).
    kept_registered: Arc<AtomicBool>,
    /// What each followed token is, from its kept contract, by token id.
    infos: Mutex<HashMap<String, TokenInfo>>,
    /// The search of the network for the wallet's tokens.
    pub(in crate::real) search: super::token_search::Search,
    /// The wallet's identities (base58), as of the last snapshot: whose tokens the search looks for.
    identities: Mutex<Vec<String>>,
}

impl TokenState {
    pub(in crate::real) fn new(path: &Path, root: &Path) -> Self {
        Self::on(crate::network::current(), path, root)
    }

    /// On `network`: everything off on mainnet (R-9), where no file is opened and nothing asked.
    pub(in crate::real) fn on(network: Network, path: &Path, root: &Path) -> Self {
        let here = network == Network::Testnet;
        TokenState {
            search: super::token_search::Search::on(here, root),
            identities: Mutex::default(),
            seen: Arc::default(),
            facts: Arc::default(),
            reading: Arc::default(),
            kept_registered: Arc::default(),
            infos: Mutex::default(),
            file: here.then(|| Arc::new(WalletTokens::new(path.to_path_buf()))),
        }
    }
}

fn off() -> CoreError {
    CoreError::NotAvailable(NO_TOKENS.into())
}

/// Keep what a search found, for [`Real::track_token`].
fn remember(seen: &Mutex<Seen>, found: &chain::FoundTokens) {
    let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    for token in &found.tokens {
        seen.tokens.insert(token.token_id.clone(), token.clone());
    }
    for (id, bytes) in &found.contracts {
        seen.contracts.insert(id.clone(), bytes.clone());
    }
}

/// Follow `token_id` by hand: it, and its proved contract, go to `tokens.json`. Only a token a
/// search found can be followed: its facts come from a proof, never from the screen.
fn track(
    file: &WalletTokens,
    seen: &Mutex<Seen>,
    token_id: &str,
    identities: &[String],
) -> Result<(), CoreError> {
    // Already in the file: a hidden one comes back on the wallet's identities, keeping how it
    // came in («Вернуть в список», architect, part 4).
    if file.get(token_id).is_some() {
        for identity in identities {
            file.hide(token_id, identity, false)
                .map_err(|e| CoreError::Failed(e.to_string()))?;
        }
        return Ok(());
    }
    let (info, contract) = {
        let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
        let info =
            seen.tokens.get(token_id).cloned().ok_or_else(|| {
                CoreError::Failed("the token was not found by a search first".into())
            })?;
        let contract = seen.contracts.get(&info.contract_id).cloned();
        (info, contract)
    };
    let saved = file
        .track(Tracked {
            token_id: info.token_id.clone(),
            contract_id: info.contract_id,
            position: info.position,
            origin: Origin::ByHand,
            contract: None,
        })
        .and_then(|()| match contract {
            Some(bytes) => file.set_contract(&info.token_id, &bytes),
            None => Ok(()),
        });
    saved.map_err(|e| CoreError::Failed(e.to_string()))
}

impl TokenState {
    pub(in crate::real) fn identities(&self) -> Vec<String> {
        self.identities
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl TokenState {
    /// What the followed tokens are, from their kept contracts; decoded once per contract.
    fn infos(&self, file: &WalletTokens, tracked: &[Tracked]) -> HashMap<String, TokenInfo> {
        let mut infos = self.infos.lock().unwrap_or_else(|e| e.into_inner());
        if tracked.iter().any(|t| !infos.contains_key(&t.token_id)) {
            for (_, bytes) in file.contracts() {
                if let Some(contract) = evolite_chain::tokens::info::contract_from_bytes(&bytes) {
                    for info in evolite_chain::tokens::info::tokens_of(&contract) {
                        infos.insert(info.token_id.clone(), info);
                    }
                }
            }
        }
        infos.clone()
    }
}

impl Real {
    /// The shown identity's tokens for the snapshot, from the last proved read; a new read in
    /// the background once [`FACTS_EVERY`] has passed (AC-T1: after a restart the balances come
    /// by themselves). Mainnet: nothing (R-9).
    pub(in crate::real) fn shown_tokens(
        &self,
        identity: Option<&str>,
        identities: Vec<String>,
    ) -> evolite_api::tokens::Tokens {
        use evolite_api::tokens::Tokens;
        let Some(file) = &self.tokens.file else {
            return Tokens::off();
        };
        *self
            .tokens
            .identities
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = identities;
        if self.tokens.search.due() {
            self.find_my_tokens(false);
        }
        let tracked = file.all();
        let infos = self.tokens.infos(file, &tracked);
        let (rows, due) = {
            let facts = self.tokens.facts.lock().unwrap_or_else(|e| e.into_inner());
            let current = facts
                .as_ref()
                .filter(|(who, _, _)| Some(who.as_str()) == identity);
            let due = current.is_none_or(|(_, at, _)| at.elapsed() >= FACTS_EVERY);
            let rows = match identity {
                Some(id) => rows(&tracked, &infos, current.map(|(_, _, f)| f), |token| {
                    file.is_hidden(token, id)
                }),
                None => Vec::new(),
            };
            (rows, due)
        };
        if let Some(id) = identity
            && due
            && !tracked.is_empty()
            && self.chain.platform_ready()
        {
            let ids = tracked.iter().map(|t| t.token_id.clone()).collect();
            self.read_token_facts(id.to_owned(), ids, file.clone());
        }
        Tokens {
            rows,
            search: self.tokens.search.shown(),
            hidden: identity
                .map(|id| {
                    tracked
                        .iter()
                        .filter(|t| file.is_hidden(&t.token_id, id))
                        .map(|t| t.token_id.clone())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// One read of the facts of `tokens` on `identity`, unless one is under way. A failed read
    /// keeps what was known, and is tried again after [`FACTS_EVERY`].
    fn read_token_facts(&self, identity: String, tokens: Vec<String>, file: Arc<WalletTokens>) {
        if self.tokens.reading.swap(true, Ordering::AcqRel) {
            return;
        }
        let (chain, refresh) = (self.chain.clone(), self.refresher());
        let (facts, reading) = (self.tokens.facts.clone(), self.tokens.reading.clone());
        let kept_registered = self.tokens.kept_registered.clone();
        tokio::spawn(async move {
            if !kept_registered.swap(true, Ordering::AcqRel) {
                let kept: Vec<Vec<u8>> = file.contracts().into_iter().map(|(_, b)| b).collect();
                chain.register_kept_contracts(&kept).await;
            }
            let read = {
                let (chain, identity) = (chain.clone(), identity.clone());
                async move { chain.token_facts(&identity, &tokens).await }
            };
            let result = read_within(FACTS_WAIT, read).await;
            {
                let mut facts = facts.lock().unwrap_or_else(|e| e.into_inner());
                let now = std::time::Instant::now();
                match result {
                    Ok(read) => *facts = Some((identity, now, read)),
                    Err(_) => match facts.as_mut() {
                        Some((who, at, _)) if *who == identity => *at = now,
                        _ => *facts = Some((identity, now, chain::TokenFacts::default())),
                    },
                }
            }
            reading.store(false, Ordering::Release);
            refresh();
        });
    }

    /// "Add token" by a token's or a contract's id: answered with [`Event::TokenFound`].
    pub(in crate::real) fn find_token(&mut self, id: String) -> (Option<Event>, bool) {
        self.search(Search::Id, id)
    }

    /// "Add token" by a keyword: answered with [`Event::TokenFound`].
    pub(in crate::real) fn search_tokens(&mut self, keyword: String) -> (Option<Event>, bool) {
        self.search(Search::Keyword, keyword)
    }

    fn search(&mut self, how: Search, query: String) -> (Option<Event>, bool) {
        if self.tokens.file.is_none() {
            let event = Event::TokenFound {
                query,
                result: Err(off()),
            };
            return (Some(event), false);
        }
        let not_synced = |query: &str| Event::TokenFound {
            query: query.to_owned(),
            result: Err(CoreError::NotSynced),
        };
        if !self.chain.platform_ready() {
            let _ = self.out.send(not_synced(&query));
        }
        let not_synced = not_synced(&query);
        let chain = self.chain.clone();
        let seen = self.tokens.seen.clone();
        let id = query;
        self.answer_when_platform_ready("task:find_token", not_synced, async move {
            let read = {
                let id = id.clone();
                async move {
                    match how {
                        Search::Id => chain.find_token(&id).await,
                        Search::Keyword => chain.search_tokens(&id).await,
                    }
                }
            };
            let result = match read_within(FIND_WAIT, read).await {
                Ok(found) => {
                    remember(&seen, &found);
                    Ok(found.tokens)
                }
                Err(e) => Err(CoreError::Failed(e)),
            };
            Event::TokenFound { query: id, result }
        });
        (None, false)
    }

    /// Follow a token found by a search (`on`), or stop following it.
    pub(in crate::real) fn track_token(
        &mut self,
        token_id: String,
        on: bool,
    ) -> (Option<Event>, bool) {
        let result = match &self.tokens.file {
            None => Err(off()),
            Some(file) if on => track(
                file,
                &self.tokens.seen,
                &token_id,
                &self.tokens.identities(),
            ),
            Some(file) => untrack(file, &token_id, &self.tokens.identities()),
        };
        let changed = result.is_ok();
        if changed {
            // A token just added is read with the next snapshot, not a minute later.
            let mut facts = self.tokens.facts.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, at, _)) = facts.as_mut()
                && let Some(earlier) = at.checked_sub(FACTS_EVERY)
            {
                *at = earlier;
            }
        }
        (
            Some(Event::Done {
                what: DoneWhat::Token,
                result,
            }),
            changed,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(token: &str, contract: &str) -> TokenInfo {
        TokenInfo {
            token_id: token.into(),
            contract_id: contract.into(),
            position: 0,
            names: vec![("en".into(), "coffee".into())],
            decimals: 2,
            issuer: "I".into(),
            issuer_name: None,
            transferable: true,
            keeps_history: false,
        }
    }

    fn file(name: &str) -> WalletTokens {
        let dir =
            std::env::temp_dir().join(format!("evolite-core-tokens-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        WalletTokens::new(dir.join("tokens.json"))
    }

    #[test]
    fn a_token_found_is_followed_with_its_proved_contract() {
        let seen = Mutex::new(Seen::default());
        remember(
            &seen,
            &chain::FoundTokens {
                tokens: vec![info("T", "C")],
                contracts: vec![("C".into(), vec![1, 2, 3])],
            },
        );
        let file = file("found");
        track(&file, &seen, "T", &[]).unwrap();
        let kept = file.get("T").expect("followed");
        assert_eq!(
            (kept.contract_id.as_str(), kept.origin),
            ("C", Origin::ByHand)
        );
        assert_eq!(file.contracts(), [("C".to_owned(), vec![1, 2, 3])]);
    }

    #[test]
    fn a_token_no_search_found_is_not_followed() {
        let seen = Mutex::new(Seen::default());
        let file = file("unseen");
        assert!(matches!(
            track(&file, &seen, "T", &[]),
            Err(CoreError::Failed(_))
        ));
        assert!(file.all().is_empty());
    }
}

/// The rows of the shown identity: the tokens followed, less the ones hidden on it (R-11);
/// a token the search found is left out at zero, one added by hand is not. What the last read
/// did not prove stays `None`, never 0 (AC-T5). A token whose contract cannot be read from
/// `tokens.json` is left out until it is read again.
fn rows(
    tracked: &[Tracked],
    infos: &HashMap<String, TokenInfo>,
    facts: Option<&chain::TokenFacts>,
    hidden: impl Fn(&str) -> bool,
) -> Vec<evolite_api::tokens::TokenRow> {
    use evolite_api::tokens::{TokenOrigin, TokenRow};
    tracked
        .iter()
        .filter(|t| !hidden(&t.token_id))
        .filter_map(|t| {
            let info = infos.get(&t.token_id)?.clone();
            let id = &t.token_id;
            let balance = facts.and_then(|f| f.balances.get(id).copied());
            let origin = match t.origin {
                Origin::Found => TokenOrigin::Found,
                Origin::ByHand => TokenOrigin::ByHand,
            };
            if origin == TokenOrigin::Found && balance == Some(0) {
                return None;
            }
            Some(TokenRow {
                info,
                origin,
                balance,
                paused: facts.and_then(|f| f.paused.get(id).copied()),
                frozen: facts.and_then(|f| f.frozen.get(id).copied()),
                // The claim flag needs the distribution's last claim (stage 1, R-11); not read
                // yet in this part.
                claim: false,
            })
        })
        .collect()
}

#[cfg(test)]
mod rows_tests {
    use super::*;
    use evolite_api::tokens::TokenOrigin;

    fn tracked(id: &str, origin: Origin) -> Tracked {
        Tracked {
            token_id: id.into(),
            contract_id: "C".into(),
            position: 0,
            origin,
            contract: None,
        }
    }

    fn info(id: &str) -> TokenInfo {
        TokenInfo {
            token_id: id.into(),
            contract_id: "C".into(),
            position: 0,
            names: vec![("en".into(), id.into())],
            decimals: 0,
            issuer: "I".into(),
            issuer_name: None,
            transferable: true,
            keeps_history: false,
        }
    }

    #[test]
    fn rows_of_the_shown_identity() {
        let list = [
            tracked("A", Origin::ByHand),
            tracked("B", Origin::Found),
            tracked("Z", Origin::Found),
            tracked("H", Origin::ByHand),
            tracked("N", Origin::ByHand),
            tracked("U", Origin::ByHand),
        ];
        let infos: HashMap<String, TokenInfo> = ["A", "B", "Z", "H", "N"]
            .map(|i| (i.to_owned(), info(i)))
            .into();
        let mut facts = chain::TokenFacts::default();
        for (id, balance) in [("A", 0), ("B", 7), ("Z", 0), ("H", 3)] {
            facts.balances.insert(id.into(), balance);
            facts.paused.insert(id.into(), id == "B");
            facts.frozen.insert(id.into(), false);
        }
        let shown = rows(&list, &infos, Some(&facts), |id| id == "H");
        let ids: Vec<&str> = shown.iter().map(|r| r.info.token_id.as_str()).collect();
        assert_eq!(
            ids,
            ["A", "B", "N"],
            "H hidden, Z found at zero, U not readable"
        );
        assert_eq!(
            (shown[0].balance, shown[0].origin),
            (Some(0), TokenOrigin::ByHand)
        );
        assert_eq!(
            (shown[1].paused, shown[1].origin),
            (Some(true), TokenOrigin::Found)
        );
        assert_eq!(
            (shown[2].balance, shown[2].paused, shown[2].frozen),
            (None, None, None),
            "not read yet: not known, never zero"
        );
        let unread = rows(&list, &infos, None, |_| false);
        assert!(unread.iter().all(|r| r.balance.is_none()));
        assert!(
            unread.iter().any(|r| r.info.token_id == "Z"),
            "a found token is not left out before its balance is known"
        );
    }
}

/// Whom a token goes to: an `@name` or an identity id, as credits (АРХ-5).
#[derive(Debug, PartialEq)]
enum Recipient {
    Name(String),
    Id(Identifier),
}

fn recipient(to: &str) -> Result<Recipient, CoreError> {
    let to = to.trim();
    if to.starts_with('@') {
        return Ok(Recipient::Name(dpns::label(to)?));
    }
    Identifier::from_string(to, Encoding::Base58)
        .map(Recipient::Id)
        .map_err(|_| CoreError::BadAddress)
}

#[cfg(test)]
mod recipient_tests {
    use super::*;

    #[test]
    fn a_token_goes_to_a_name_or_an_identity_id() {
        assert_eq!(recipient(" @Alice "), Ok(Recipient::Name("alice".into())));
        let id = Identifier::from([7u8; 32]);
        assert_eq!(
            recipient(&id.to_string(Encoding::Base58)),
            Ok(Recipient::Id(id))
        );
        assert_eq!(
            recipient("yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh"),
            Err(CoreError::BadAddress)
        );
        assert_eq!(recipient(""), Err(CoreError::BadAddress));
        assert!(matches!(recipient("@a"), Err(CoreError::BadName)));
    }
}

/// How long a token transfer may take before the answer says it timed out; the transfer may
/// still land, and the next read of the balances shows it.
const SEND_WAIT: Duration = Duration::from_secs(120);

impl Real {
    /// Sends `amount` of a followed token (Spend::SendToken), past the password gate. The
    /// refusals before the password are the screen's (`send_refusal`, AC-T3); here only what
    /// cannot be sent at all. Answered with `Done { what: DoneWhat::SendToken }`; the sender's
    /// balance is taken from the proved answer.
    pub(in crate::real) fn send_token(&self, token_id: String, to: String, amount: u64) -> Event {
        let refused = |e| Event::Done {
            what: DoneWhat::SendToken,
            result: Err(e),
        };
        let Some(file) = self.tokens.file.clone() else {
            return refused(off());
        };
        if amount == 0 {
            return refused(CoreError::BadAmount);
        }
        let Some(token) = file.get(&token_id) else {
            return refused(CoreError::Failed("the token is not in the list".into()));
        };
        let Some(info) = self
            .tokens
            .infos(&file, &file.all())
            .get(&token_id)
            .cloned()
        else {
            return refused(CoreError::Failed(
                "the token's contract is not read yet".into(),
            ));
        };
        let to = match recipient(&to) {
            Ok(to) => to,
            Err(e) => return refused(e),
        };
        if !self.chain.platform_ready() {
            return refused(CoreError::NotSynced);
        }
        let Some((id, keys)) = self
            .unlocked
            .as_ref()
            .and_then(|open| Some((open.id, open.keys.clone()?)))
        else {
            return refused(CoreError::NotAvailable(tr!("The keys are not open").into()));
        };
        let (chain, facts) = (self.chain.clone(), self.tokens.facts.clone());
        let known = facts.lock().unwrap_or_else(|e| e.into_inner()).clone();
        self.answer_later("task:send_token", async move {
            let result = async {
                let to = match to {
                    Recipient::Id(id) => id,
                    Recipient::Name(label) => match chain.resolve_name(&label).await {
                        Ok(Some(id)) => Identifier::from_string(&id, Encoding::Base58)
                            .map_err(|e| CoreError::Failed(e.to_string()))?,
                        Ok(None) => return Err(CoreError::NameNotFound),
                        Err(e) => return Err(read_failure(e)),
                    },
                };
                let wallet = loaded(&chain, &id).await?;
                // Review r1, Should 1: what is known of the token on the sender, before the fee.
                let me = acting_identity(&wallet)
                    .await
                    .ok_or(CoreError::NotAvailable(
                        tr!("The wallet has no identity").into(),
                    ))?;
                let read = {
                    let (chain, me, tokens) = (chain.clone(), me.clone(), vec![token_id.clone()]);
                    move || async move { chain.token_facts(&me, &tokens).await }
                };
                check_before_signing(known, &me, &info, amount, read).await?;
                let send = chain.send_token(
                    &wallet,
                    &keys,
                    &token.contract_id,
                    token.position,
                    to,
                    amount,
                );
                let balance = tokio::time::timeout(SEND_WAIT, send)
                    .await
                    .map_err(|_| CoreError::Failed("timed out".into()))?
                    .map_err(dashpay_failure)?;
                // The sender's balance from the proof, so the row is right before the next read.
                let mut facts = facts.lock().unwrap_or_else(|e| e.into_inner());
                if let (Some(balance), Some((_, _, f))) = (balance, facts.as_mut()) {
                    f.balances.insert(token.token_id.clone(), balance);
                }
                Ok(())
            }
            .await;
            Event::Done {
                what: DoneWhat::SendToken,
                result,
            }
        });
        Event::Progress {
            what: "send_token",
            fraction: 0.0,
        }
    }
}

/// Stop following `token_id`. One added by hand leaves the list; one the search found is
/// hidden on every identity of the wallet instead, so the next pass does not bring it back
/// (`Command::TrackToken`).
fn untrack(file: &WalletTokens, token_id: &str, identities: &[String]) -> Result<(), CoreError> {
    let failed = |e: std::io::Error| CoreError::Failed(e.to_string());
    match file.get(token_id) {
        Some(t) if t.origin == Origin::Found => {
            for identity in identities {
                file.hide(token_id, identity, true).map_err(failed)?;
            }
            Ok(())
        }
        _ => file.untrack(token_id).map_err(failed),
    }
}

#[cfg(test)]
mod untrack_tests {
    use super::*;

    #[test]
    fn a_found_token_taken_out_is_hidden_and_one_by_hand_leaves() {
        let dir = std::env::temp_dir().join(format!("evolite-untrack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = WalletTokens::new(dir.join("tokens.json"));
        for (id, origin) in [("F", Origin::Found), ("H", Origin::ByHand)] {
            file.track(Tracked {
                token_id: id.into(),
                contract_id: "C".into(),
                position: 0,
                origin,
                contract: None,
            })
            .unwrap();
        }
        let identities = ["I1".to_owned(), "I2".to_owned()];
        untrack(&file, "F", &identities).unwrap();
        untrack(&file, "H", &identities).unwrap();
        assert!(file.get("F").is_some(), "kept, so the search knows it");
        assert!(file.is_hidden("F", "I1") && file.is_hidden("F", "I2"));
        assert!(file.get("H").is_none());
    }
}

/// The last proved facts the core keeps: of which identity, when read, and what.
type KnownFacts = Option<(String, std::time::Instant, chain::TokenFacts)>;

/// Review r1, Should 1: before signing, what the core already knows about `info` on
/// `identity` — its balance, pause and freeze — must allow sending `amount`; Platform's refusal
/// of these comes after the fee is taken. Facts of another identity, older than
/// [`FACTS_EVERY`] or without this token are read first (`read`, proved), never skipped.
async fn check_before_signing<F>(
    known: KnownFacts,
    identity: &str,
    info: &TokenInfo,
    amount: u64,
    read: impl FnOnce() -> F,
) -> Result<(), CoreError>
where
    F: Future<Output = Result<chain::TokenFacts, String>>,
{
    use evolite_api::tokens::{Sender, TokenOrigin, TokenRefusal, TokenRow, send_refusal};
    let id = &info.token_id;
    let fresh = known.filter(|(who, at, f)| {
        who == identity && at.elapsed() < FACTS_EVERY && f.balances.contains_key(id)
    });
    let facts = match fresh {
        Some((_, _, f)) => f,
        None => read().await.map_err(read_failure)?,
    };
    let row = TokenRow {
        info: info.clone(),
        origin: TokenOrigin::ByHand,
        balance: facts.balances.get(id).copied(),
        paused: facts.paused.get(id).copied(),
        frozen: facts.frozen.get(id).copied(),
        claim: false,
    };
    // Only the token's facts here: the key is platform-wallet's choice, the credits Platform's.
    let sender = Sender {
        has_key: true,
        credits: Some(u64::MAX),
        fee: 0,
    };
    // The screen's words (`screens/tokens/state.rs`): one key, one translation.
    let words = |w: &str| Err(CoreError::NotAvailable(w.to_owned()));
    match send_refusal(&row, Some(amount), sender) {
        None => Ok(()),
        Some(TokenRefusal::Paused) => words(tr!(
            "The issuer paused the token — transfers are not available now."
        )),
        Some(TokenRefusal::Frozen) => {
            words(tr!("The issuer froze this balance — it cannot be sent."))
        }
        Some(TokenRefusal::NotTransferable) => words(tr!(
            "This token cannot be transferred: its issuer decided so."
        )),
        Some(TokenRefusal::NotKnown) => words(tr!("The balance is not known yet: wait for it.")),
        Some(TokenRefusal::Zero) => Err(CoreError::BadAmount),
        Some(TokenRefusal::MoreThanBalance(_)) => Err(CoreError::NotEnough),
        Some(TokenRefusal::NoKey | TokenRefusal::NoCredits) => Ok(()),
    }
}

#[cfg(test)]
mod signing_tests {
    use super::*;
    use std::cell::Cell;

    fn info() -> TokenInfo {
        TokenInfo {
            token_id: "T".into(),
            contract_id: "C".into(),
            position: 0,
            names: vec![("en".into(), "coffee".into())],
            decimals: 2,
            issuer: "I".into(),
            issuer_name: None,
            transferable: true,
            keeps_history: false,
        }
    }

    fn facts(balance: u64, paused: bool, frozen: bool) -> chain::TokenFacts {
        let mut f = chain::TokenFacts::default();
        f.balances.insert("T".into(), balance);
        f.paused.insert("T".into(), paused);
        f.frozen.insert("T".into(), frozen);
        f
    }

    fn known(who: &str, age: Duration, f: chain::TokenFacts) -> KnownFacts {
        Some((who.into(), std::time::Instant::now() - age, f))
    }

    /// Runs the check; how many reads it asked for.
    async fn check(
        known: KnownFacts,
        read_gives: chain::TokenFacts,
        i: &TokenInfo,
        amount: u64,
    ) -> (Result<(), CoreError>, u32) {
        let reads = Cell::new(0);
        let result = check_before_signing(known, "ME", i, amount, || {
            reads.set(reads.get() + 1);
            std::future::ready(Ok(read_gives))
        })
        .await;
        (result, reads.get())
    }

    #[tokio::test]
    async fn fresh_facts_of_the_sender_refuse_without_asking_the_network() {
        let i = info();
        let fresh = |f| known("ME", Duration::from_secs(5), f);
        let ok = facts(500, false, false);
        let (paused, reads) = check(fresh(facts(500, true, false)), ok.clone(), &i, 100).await;
        assert!(
            matches!(paused, Err(CoreError::NotAvailable(_))),
            "{paused:?}"
        );
        assert_eq!(reads, 0, "nothing asked of the network");
        let (frozen, _) = check(fresh(facts(500, false, true)), ok.clone(), &i, 100).await;
        assert!(matches!(frozen, Err(CoreError::NotAvailable(_))));
        let (more, _) = check(fresh(facts(50, false, false)), ok.clone(), &i, 100).await;
        assert_eq!(more, Err(CoreError::NotEnough));
        let fixed = TokenInfo {
            transferable: false,
            ..info()
        };
        let (fixed, _) = check(fresh(ok.clone()), ok.clone(), &fixed, 100).await;
        assert!(matches!(fixed, Err(CoreError::NotAvailable(_))));
        let (fine, reads) = check(fresh(ok.clone()), ok, &i, 100).await;
        assert_eq!((fine, reads), (Ok(()), 0));
    }

    #[tokio::test]
    async fn old_missing_or_another_identitys_facts_are_read_first() {
        let i = info();
        let paused_now = facts(500, true, false);
        for stale in [
            None,
            known(
                "ME",
                FACTS_EVERY + Duration::from_secs(1),
                facts(500, false, false),
            ),
            known("OTHER", Duration::from_secs(1), facts(500, false, false)),
            known("ME", Duration::from_secs(1), chain::TokenFacts::default()),
        ] {
            let (result, reads) = check(stale, paused_now.clone(), &i, 100).await;
            assert_eq!(reads, 1, "read first");
            assert!(
                matches!(result, Err(CoreError::NotAvailable(_))),
                "the read says paused"
            );
        }
        // A read that fails sends nothing.
        let failed = check_before_signing(None, "ME", &i, 100, || {
            std::future::ready(Err::<chain::TokenFacts, _>("no answer".to_owned()))
        })
        .await;
        assert!(failed.is_err());
    }
}

#[cfg(test)]
mod back_tests {
    use super::*;

    /// Architect, part 4: «Вернуть в список». A found token hidden on the wallet's identities
    /// comes back with TrackToken on, keeping how it came in; no new search is needed.
    #[test]
    fn a_hidden_token_comes_back_with_its_origin() {
        let dir = std::env::temp_dir().join(format!("evolite-back-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = WalletTokens::new(dir.join("tokens.json"));
        file.track(Tracked {
            token_id: "T".into(),
            contract_id: "C".into(),
            position: 0,
            origin: Origin::Found,
            contract: None,
        })
        .unwrap();
        let identities = ["I1".to_owned(), "I2".to_owned()];
        untrack(&file, "T", &identities).unwrap();
        assert!(file.is_hidden("T", "I1") && file.is_hidden("T", "I2"));
        // Nothing found by a search in this run: the token is known from the file.
        let seen = Mutex::new(Seen::default());
        track(&file, &seen, "T", &identities).unwrap();
        assert!(
            !file.is_hidden("T", "I1") && !file.is_hidden("T", "I2"),
            "visible again"
        );
        assert_eq!(
            file.get("T").map(|t| t.origin),
            Some(Origin::Found),
            "origin kept"
        );
    }
}
