//! The names under a field where another person's @name is typed (FEAT-0039): DPNS names by
//! their beginning, proved, with the profiles' names of the strangers shown — and a cache for
//! the session, so a name typed letter by letter costs one or two reads, not one per letter.
//!
//! The cache is in memory, of the open wallet: the lock clears it, nothing goes to disk.

use std::collections::HashSet;
use std::time::Instant;

use super::names;
use crate::diag::Record;
use crate::real::*;
use chain::{FoundName, NAMES_PER_SEARCH};

/// How long a search's answer, and a profile's name, are taken from the cache (architect §3).
const KEEP: Duration = Duration::from_secs(5 * 60);
/// The most searches the cache keeps; the oldest goes first.
const MOST: usize = 200;
/// How long the network may take with the names, and then with the profiles (analyst: 5 s).
const WAIT: Duration = Duration::from_secs(5);
/// How many strangers' profiles are read: the most rows a list shows.
const SHOWN: usize = 5;

/// A search's answer as kept.
struct Kept {
    names: Vec<FoundName>,
    at: Instant,
}

/// The session's searches and the strangers' profile names, and the search under way.
#[derive(Default)]
pub(in crate::real) struct NameSearch {
    /// By the beginning asked, in DPNS's homograph-safe form.
    searches: HashMap<String, Kept>,
    /// The profile's name by identity (base58); `None`: read, no name.
    profiles: HashMap<String, (Option<String>, Instant)>,
    /// The search under way: a new one stops it (architect §3).
    running: Option<tokio::task::AbortHandle>,
}

impl NameSearch {
    /// The names for `prefix` without asking the network: its own answer, or — narrowed — the
    /// answer to a shorter beginning that was not full (fewer than [`NAMES_PER_SEARCH`] names
    /// are all there are on it). `None`: the network has to be asked.
    fn cached(&self, prefix: &str, now: Instant) -> Option<Vec<FoundName>> {
        let fresh = |k: &Kept| now.saturating_duration_since(k.at) < KEEP;
        if let Some(kept) = self.searches.get(prefix).filter(|k| fresh(k)) {
            return Some(kept.names.clone());
        }
        self.searches
            .iter()
            .filter(|(asked, kept)| {
                prefix.starts_with(asked.as_str())
                    && fresh(kept)
                    && kept.names.len() < NAMES_PER_SEARCH as usize
            })
            .max_by_key(|(asked, _)| asked.len())
            .map(|(_, kept)| {
                kept.names
                    .iter()
                    .filter(|n| n.normalized.starts_with(prefix))
                    .cloned()
                    .collect()
            })
    }

    /// Keeps the network's answer for `prefix`; past [`MOST`] the oldest goes.
    fn keep(&mut self, prefix: String, names: Vec<FoundName>, now: Instant) {
        self.searches.insert(prefix, Kept { names, at: now });
        while self.searches.len() > MOST {
            let oldest = self
                .searches
                .iter()
                .min_by_key(|(_, k)| k.at)
                .map(|(p, _)| p.clone());
            match oldest {
                Some(p) => self.searches.remove(&p),
                None => break,
            };
        }
    }

    /// The profile's name of `id` if read in the last [`KEEP`]: `Some(None)` read and nameless.
    fn profile(&self, id: &str, now: Instant) -> Option<Option<String>> {
        self.profiles
            .get(id)
            .filter(|(_, at)| now.saturating_duration_since(*at) < KEEP)
            .map(|(name, _)| name.clone())
    }

    /// Everything goes: the lock, another wallet.
    pub(in crate::real) fn clear(&mut self) {
        if let Some(running) = self.running.take() {
            running.abort();
        }
        self.searches.clear();
        self.profiles.clear();
    }
}

/// The rows of an answer, with the profiles' names known so far.
fn rows(names: &[FoundName], search: &NameSearch, now: Instant) -> Vec<NameSuggestion> {
    names
        .iter()
        .map(|n| NameSuggestion {
            label: n.label.clone(),
            identity_id: n.identity_id.clone(),
            display_name: search.profile(&n.identity_id, now).flatten(),
        })
        .collect()
}

/// The strangers a list will show — not contacts, shorter names first, then by the name — whose
/// profile is not known yet: at most [`SHOWN`]. Contacts' names come from the snapshot.
fn unknown_strangers(
    names: &[FoundName],
    contacts: &HashSet<String>,
    search: &NameSearch,
    now: Instant,
) -> Vec<String> {
    let mut strangers: Vec<&FoundName> = names
        .iter()
        .filter(|n| !contacts.contains(&n.identity_id))
        .collect();
    strangers.sort_by(|a, b| {
        (a.normalized.len(), &a.normalized).cmp(&(b.normalized.len(), &b.normalized))
    });
    let mut ids: Vec<String> = Vec::new();
    for n in strangers {
        if ids.len() == SHOWN {
            break;
        }
        if !ids.contains(&n.identity_id) {
            ids.push(n.identity_id.clone());
        }
    }
    ids.retain(|id| search.profile(id, now).is_none());
    ids
}

impl Real {
    /// [`Command::SuggestNames`]: from the cache at once when it has the answer, else from the
    /// network within [`WAIT`] — and then the strangers' profile names in a second answer.
    pub(in crate::real) fn suggest_names(&mut self, prefix: String, seq: u64) -> Option<Event> {
        let answer = move |result| Event::NameSuggestions { seq, result };
        let Some(prefix) = evolite_common::dpns::search_key(&prefix)
            .filter(|p| p.len() >= evolite_common::dpns::SEARCH_FROM)
        else {
            return Some(answer(Ok(Vec::new())));
        };
        let search = self.name_search.clone();
        let mut state = search.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(running) = state.running.take() {
            running.abort();
        }
        let now = Instant::now();
        let contacts: HashSet<String> = self
            .contacts
            .iter()
            .map(|c| c.identity_id.clone())
            .collect();
        let cached = state.cached(&prefix, now);
        if cached.is_none() && !self.chain.platform_ready() {
            // Platform is still connecting: the screen shows its own rows and says so at once.
            return Some(answer(Err(CoreError::NotSynced)));
        }
        let first = cached.as_ref().map(|names| {
            crate::diag::record(Record::NameSearch {
                found: names.len() as u32,
                cached: true,
            });
            answer(Ok(rows(names, &state, now)))
        });
        let chain = self.chain.clone();
        let (out, wake) = (self.out.clone(), self.wake.clone());
        let shared = search.clone();
        let task = tokio::spawn(async move {
            let names = match cached {
                Some(names) => names,
                None => {
                    let read = {
                        let (chain, prefix) = (chain.clone(), prefix.clone());
                        async move { chain.search_names(&prefix).await }
                    };
                    match names::read_within(WAIT, read).await {
                        Ok(names) => {
                            crate::diag::record(Record::NameSearch {
                                found: names.len() as u32,
                                cached: false,
                            });
                            let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                            state.keep(prefix, names.clone(), Instant::now());
                            let _ = out.send(answer(Ok(rows(&names, &state, Instant::now()))));
                            drop(state);
                            wake();
                            names
                        }
                        Err(e) => {
                            let _ = out.send(answer(Err(read_failure(e))));
                            wake();
                            return;
                        }
                    }
                }
            };
            let unknown = {
                let state = shared.lock().unwrap_or_else(|e| e.into_inner());
                unknown_strangers(&names, &contacts, &state, Instant::now())
            };
            let ids: Vec<Identifier> = unknown
                .iter()
                .filter_map(|id| Identifier::from_string(id, Encoding::Base58).ok())
                .collect();
            if ids.is_empty() {
                return;
            }
            // Without the names in time the rows stay as they are, without a word (architect §2).
            let read = async move { chain.profiles_of(&ids).await };
            let Ok(profiles) = names::read_within(WAIT, read).await else {
                return;
            };
            let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            for id in &unknown {
                let name = Identifier::from_string(id, Encoding::Base58)
                    .ok()
                    .and_then(|i| profiles.get(&i))
                    .and_then(|p| p.display_name.clone());
                state.profiles.insert(id.clone(), (name, now));
            }
            let _ = out.send(answer(Ok(rows(&names, &state, now))));
            drop(state);
            wake();
        });
        state.running = Some(task.abort_handle());
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(label: &str, id: &str) -> FoundName {
        FoundName {
            label: label.into(),
            normalized: evolite_common::dpns::search_key(label).unwrap(),
            identity_id: id.into(),
        }
    }

    fn some(n: usize) -> Vec<FoundName> {
        (0..n)
            .map(|i| found(&format!("ali{i}"), &format!("id{i}")))
            .collect()
    }

    /// AC-T3: an answer that was not full is all there is on its beginning: «alice» is
    /// narrowed from «ali» without the network; a full one is asked again.
    #[test]
    fn a_longer_beginning_is_narrowed_from_an_answer_that_was_not_full() {
        let now = Instant::now();
        let mut s = NameSearch::default();
        s.keep(
            "a11".into(),
            vec![
                found("alice", "a"),
                found("alex", "b"),
                found("AIice-pay", "c"),
            ],
            now,
        );
        let narrowed = s.cached("a11ce", now).expect("narrowed");
        let labels: Vec<_> = narrowed.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["alice", "AIice-pay"]);
        // Full: the network has more on «ali» than it said.
        let mut s = NameSearch::default();
        s.keep("a11".into(), some(NAMES_PER_SEARCH as usize), now);
        assert!(s.cached("a11c", now).is_none());
        assert!(s.cached("a11", now).is_some(), "its own answer is kept");
    }

    /// AC-T3: five minutes, then the network again.
    #[test]
    fn an_answer_is_kept_five_minutes() {
        let now = Instant::now();
        let mut s = NameSearch::default();
        s.keep("b0b".into(), vec![found("bob", "x")], now);
        assert!(s.cached("b0b", now + Duration::from_secs(299)).is_some());
        assert!(s.cached("b0b", now + KEEP).is_none());
        assert!(s.cached("b0bby", now + KEEP).is_none());
    }

    /// AC-T3: at most 200 answers, the oldest goes.
    #[test]
    fn the_cache_keeps_two_hundred_answers() {
        let now = Instant::now();
        let mut s = NameSearch::default();
        for i in 0..=MOST {
            s.keep(
                format!("p{i:03}"),
                some(NAMES_PER_SEARCH as usize),
                now + Duration::from_millis(i as u64),
            );
        }
        assert_eq!(s.searches.len(), MOST);
        assert!(!s.searches.contains_key("p000"), "the oldest went");
        assert!(s.searches.contains_key(&format!("p{MOST:03}")));
    }

    /// AC-T3: the lock clears it all.
    #[test]
    fn clearing_forgets_the_answers_and_the_profiles() {
        let now = Instant::now();
        let mut s = NameSearch::default();
        s.keep("b0b".into(), vec![found("bob", "x")], now);
        s.profiles.insert("x".into(), (Some("Bob".into()), now));
        s.clear();
        assert!(s.cached("b0b", now).is_none());
        assert!(s.profile("x", now).is_none());
    }

    /// AC-T2: the profiles read are of the strangers a list shows: no contact, shorter first,
    /// one per identity, five at most, none already known.
    #[test]
    fn profiles_are_read_for_the_strangers_shown_only() {
        let now = Instant::now();
        let mut s = NameSearch::default();
        s.profiles.insert("known".into(), (None, now));
        let names = vec![
            found("alexandra", "long"),
            found("alice", "contact"),
            found("ali", "short"),
            found("alice-pay", "twice"),
            found("alic", "twice"),
            found("ala", "known"),
            found("alim", "m"),
            found("alin", "n"),
            found("alio", "o"),
        ];
        let contacts = HashSet::from(["contact".to_string()]);
        let ids = unknown_strangers(&names, &contacts, &s, now);
        // Shown: ali, ala (known), alio, alic, alim — only the unknown are read.
        assert_eq!(ids, ["short", "o", "twice", "m"]);
    }
}
