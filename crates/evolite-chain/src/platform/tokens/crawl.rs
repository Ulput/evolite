//! The wallet's own search of the network for tokens (FEAT-0019 UC-1а, R-11), as a state that
//! says what to ask next and takes the answers. No network here: the caller asks Platform with
//! proofs (`getDataContractsByRange`, `getDataContractsLatestVersions`, the contracts) and hands
//! back only what was proved. The whole state is [`Cache`], kept on disk between runs, so a pass
//! cut short (a lock, another wallet, no network) goes on from its last id, not from the start.
//!
//! A pass: every contract id page by page after the last one (`start_after`); ids not seen
//! before are to be read. Then the latest versions of every contract read: a grown version is
//! read again, with tokens or without, as an update may bring a token into a contract that had
//! none (dpp `validate_update` v1; review r1, Should 1). Then the contracts to read, in batches.

use std::collections::{BTreeMap, BTreeSet};

/// A contract's id.
pub type Id = [u8; 32];

/// How much one request asks for.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Ids in one page of `getDataContractsByRange`.
    pub ids_page: u32,
    /// Contracts (or versions) in one request.
    pub batch: usize,
}

/// A contract with tokens, as last read: its version and what the caller keeps of its tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known<P> {
    pub version: u32,
    pub tokens: P,
}

/// Where a pass stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pass {
    /// No pass under way.
    Idle,
    /// Reading ids after `after` (`None`: from the first).
    Ids { after: Option<Id> },
    /// Asking the versions of the contracts read, after `after`.
    Versions { after: Option<Id> },
    /// Reading the contracts to read.
    Contracts,
}

/// The search's whole state, kept for the network (not for a wallet: it is public data).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cache<P> {
    /// Every contract id seen, with the version last read; `None`: not read (yet, or the proof
    /// said it is not there).
    pub ids: BTreeMap<Id, Option<u32>>,
    /// The contracts with tokens.
    pub tokens: BTreeMap<Id, Known<P>>,
    /// Contracts to read in the pass under way: new ones and ones whose version grew.
    pub to_read: BTreeSet<Id>,
    pub pass: Pass,
    /// Whole passes done.
    pub passes: u32,
}

/// The next request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    Ids { after: Option<Id>, limit: u32 },
    Versions(Vec<Id>),
    Contracts(Vec<Id>),
}

/// A contract as read with its proof: its version and its tokens (`None`: it has none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read<P> {
    pub id: Id,
    pub version: u32,
    pub tokens: Option<P>,
}

/// The proved answer to an [`Ask`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply<P> {
    Ids(Vec<Id>),
    Versions(Vec<(Id, u32)>),
    Contracts(Vec<Read<P>>),
}

/// The answer is not to the request the state asked: nothing was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotAsked;

impl<P> Default for Cache<P> {
    fn default() -> Self {
        Cache {
            ids: BTreeMap::new(),
            tokens: BTreeMap::new(),
            to_read: BTreeSet::new(),
            pass: Pass::Idle,
            passes: 0,
        }
    }
}

impl<P> Cache<P> {
    /// Start a pass, or go on with the one cut short.
    pub fn begin(&mut self) {
        if self.pass == Pass::Idle {
            self.pass = Pass::Ids { after: None };
        }
    }

    /// What to ask now; `None`: no pass under way (the last one is done).
    pub fn ask(&self, limits: Limits) -> Option<Ask> {
        match &self.pass {
            Pass::Idle => None,
            Pass::Ids { after } => Some(Ask::Ids {
                after: *after,
                limit: limits.ids_page,
            }),
            Pass::Versions { after } => Some(Ask::Versions(
                self.read_after(*after).take(limits.batch).collect(),
            )),
            Pass::Contracts => Some(Ask::Contracts(
                self.to_read.iter().take(limits.batch).copied().collect(),
            )),
        }
    }

    /// The contracts read, after `after`, in order.
    fn read_after(&self, after: Option<Id>) -> impl Iterator<Item = Id> + '_ {
        self.ids
            .iter()
            .filter(move |(k, v)| v.is_some() && after.is_none_or(|a| **k > a))
            .map(|(k, _)| *k)
    }

    /// Take the proved `reply` to `asked`, which must be what [`Cache::ask`] gives now. Nothing
    /// changes when it is not, or when the reply holds an id that was not asked about.
    pub fn take(&mut self, asked: &Ask, reply: Reply<P>, limits: Limits) -> Result<(), NotAsked> {
        if self.ask(limits).as_ref() != Some(asked) {
            return Err(NotAsked);
        }
        match (asked, reply) {
            (Ask::Ids { after, limit }, Reply::Ids(page)) => {
                let after_asked = |id: &Id| after.is_none_or(|a| *id > a);
                if page.len() > *limit as usize || !page.iter().all(after_asked) {
                    return Err(NotAsked);
                }
                for id in &page {
                    if !self.ids.contains_key(id) {
                        self.ids.insert(*id, None);
                        self.to_read.insert(*id);
                    }
                }
                // A short page is the last one.
                self.pass = match page.iter().max() {
                    Some(last) if page.len() == *limit as usize => Pass::Ids { after: Some(*last) },
                    _ => Pass::Versions { after: None },
                };
            }
            (Ask::Versions(ids), Reply::Versions(versions)) => {
                if !versions.iter().all(|(id, _)| ids.contains(id)) {
                    return Err(NotAsked);
                }
                for (id, version) in versions {
                    if self
                        .ids
                        .get(&id)
                        .copied()
                        .flatten()
                        .is_some_and(|known| version > known)
                    {
                        self.to_read.insert(id);
                    }
                }
                self.pass = Pass::Versions {
                    after: ids.last().copied(),
                };
            }
            (Ask::Contracts(ids), Reply::Contracts(read)) => {
                if !read.iter().all(|r| ids.contains(&r.id)) {
                    return Err(NotAsked);
                }
                // One the proof says is not there is not asked again either: dash-sdk's proved `FetchMany`
                // answers every id asked, `None` being a proved absence, and `crawl_step` gives only those
                // present (part 3 review r1, answer 4).
                for id in ids {
                    self.to_read.remove(id);
                }
                for r in read {
                    if let Some(known) = self.ids.get_mut(&r.id) {
                        *known = Some(r.version);
                    }
                    match r.tokens {
                        Some(tokens) => {
                            self.tokens.insert(
                                r.id,
                                Known {
                                    version: r.version,
                                    tokens,
                                },
                            );
                        }
                        None => {
                            self.tokens.remove(&r.id);
                        }
                    }
                }
            }
            _ => return Err(NotAsked),
        }
        self.settle();
        Ok(())
    }

    /// Past the stages with nothing left to ask; the pass ends when nothing is left to read.
    fn settle(&mut self) {
        loop {
            match self.pass {
                Pass::Versions { after } if self.read_after(after).next().is_none() => {
                    self.pass = Pass::Contracts;
                }
                Pass::Contracts if self.to_read.is_empty() => {
                    self.pass = Pass::Idle;
                    self.passes += 1;
                }
                _ => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        ids_page: 3,
        batch: 2,
    };

    fn id(n: u8) -> Id {
        let mut id = [0; 32];
        id[31] = n;
        id
    }

    /// A network: contract `n` with its version and, for some, a token named by it.
    #[derive(Clone)]
    struct Net {
        contracts: BTreeMap<Id, (u32, Option<String>)>,
        asked: Vec<Ask>,
    }

    impl Net {
        fn new(n: u8, with_tokens: &[u8]) -> Self {
            let contracts = (1..=n)
                .map(|i| {
                    (
                        id(i),
                        (1, with_tokens.contains(&i).then(|| format!("T{i}"))),
                    )
                })
                .collect();
            Net {
                contracts,
                asked: Vec::new(),
            }
        }

        fn answer(&mut self, ask: &Ask) -> Reply<String> {
            self.asked.push(ask.clone());
            match ask {
                Ask::Ids { after, limit } => Reply::Ids(
                    self.contracts
                        .keys()
                        .filter(|k| after.is_none_or(|a| **k > a))
                        .take(*limit as usize)
                        .copied()
                        .collect(),
                ),
                Ask::Versions(ids) => Reply::Versions(
                    ids.iter()
                        .filter_map(|i| self.contracts.get(i).map(|c| (*i, c.0)))
                        .collect(),
                ),
                Ask::Contracts(ids) => Reply::Contracts(
                    ids.iter()
                        .map(|i| {
                            let (version, tokens) = self.contracts[i].clone();
                            Read {
                                id: *i,
                                version,
                                tokens,
                            }
                        })
                        .collect(),
                ),
            }
        }
    }

    /// Run until the pass ends or `stop` asks were made.
    fn run(cache: &mut Cache<String>, net: &mut Net, stop: usize) {
        for _ in 0..stop {
            let Some(ask) = cache.ask(LIMITS) else { return };
            let reply = net.answer(&ask);
            cache
                .take(&ask, reply, LIMITS)
                .expect("the answer to its ask");
        }
    }

    fn found(cache: &Cache<String>) -> Vec<&str> {
        cache.tokens.values().map(|k| k.tokens.as_str()).collect()
    }

    #[test]
    fn a_first_pass_pages_through_every_id_and_keeps_the_contracts_with_tokens() {
        let mut net = Net::new(7, &[2, 5, 7]);
        let mut cache = Cache::default();
        assert_eq!(cache.ask(LIMITS), None, "nothing before a pass begins");
        cache.begin();
        run(&mut cache, &mut net, 100);
        assert_eq!(cache.pass, Pass::Idle);
        assert_eq!(cache.passes, 1);
        assert_eq!(cache.ids.len(), 7);
        assert_eq!(found(&cache), ["T2", "T5", "T7"]);
        assert!(cache.to_read.is_empty());
        // Pages of 3 after the last id: 1-3, 4-6, 7 (a short page ends the ids).
        let pages: Vec<Option<Id>> = net
            .asked
            .iter()
            .filter_map(|a| match a {
                Ask::Ids { after, .. } => Some(*after),
                _ => None,
            })
            .collect();
        assert_eq!(pages, [None, Some(id(3)), Some(id(6))]);
        // Every contract read once, two at a time.
        let read: Vec<usize> = net
            .asked
            .iter()
            .filter_map(|a| match a {
                Ask::Contracts(ids) => Some(ids.len()),
                _ => None,
            })
            .collect();
        assert_eq!(read, [2, 2, 2, 1]);
    }

    #[test]
    fn a_pass_cut_short_goes_on_from_its_last_id() {
        let mut net = Net::new(7, &[2, 5, 7]);
        let mut cache = Cache::default();
        cache.begin();
        run(&mut cache, &mut net, 1);
        assert_eq!(cache.pass, Pass::Ids { after: Some(id(3)) });
        // Kept on disk, the wallet locked and opened again: the same state begins again.
        let mut kept = cache.clone();
        kept.begin();
        assert_eq!(
            kept.ask(LIMITS),
            Some(Ask::Ids {
                after: Some(id(3)),
                limit: 3
            })
        );
        run(&mut kept, &mut net, 100);
        assert_eq!(found(&kept), ["T2", "T5", "T7"]);
        assert_eq!(kept.passes, 1);
        // Cut short among the contracts: the ones read are not read again.
        let mut net = Net::new(7, &[2, 5, 7]);
        let mut cache = Cache::default();
        cache.begin();
        run(&mut cache, &mut net, 4);
        assert_eq!(cache.pass, Pass::Contracts);
        let left = cache.to_read.len();
        let mut kept = cache.clone();
        kept.begin();
        let before = net.asked.len();
        run(&mut kept, &mut net, 100);
        let read_after: usize = net.asked[before..]
            .iter()
            .map(|a| match a {
                Ask::Contracts(ids) => ids.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(read_after, left);
        assert_eq!(found(&kept), ["T2", "T5", "T7"]);
    }

    #[test]
    fn a_new_pass_reads_new_contracts_and_grown_versions_only() {
        let mut net = Net::new(7, &[2, 5, 7]);
        let mut cache = Cache::default();
        cache.begin();
        run(&mut cache, &mut net, 100);
        // Two new contracts, one with a token; contract 5 updated with a second token; contract
        // 3 (no tokens) updated, read again as an update may bring a token (Should 1).
        net.contracts.insert(id(8), (1, None));
        net.contracts.insert(id(9), (1, Some("T9".into())));
        net.contracts.insert(id(5), (2, Some("T5+".into())));
        net.contracts.insert(id(3), (2, None));
        net.asked.clear();
        cache.begin();
        run(&mut cache, &mut net, 100);
        assert_eq!(cache.passes, 2);
        assert_eq!(found(&cache), ["T2", "T5+", "T7", "T9"]);
        assert_eq!(cache.tokens[&id(5)].version, 2);
        let read: BTreeSet<Id> = net
            .asked
            .iter()
            .flat_map(|a| match a {
                Ask::Contracts(ids) => ids.clone(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(read, BTreeSet::from([id(3), id(5), id(8), id(9)]));
        // The versions asked are those of every contract read, not of the new ones.
        let versions: BTreeSet<Id> = net
            .asked
            .iter()
            .flat_map(|a| match a {
                Ask::Versions(ids) => ids.clone(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(versions, (1..=7).map(id).collect::<BTreeSet<Id>>());
    }

    /// Review r1, Should 1: an update may bring a token into a contract that had none (dpp
    /// `validate_update` v1), so a grown version is read again whether it had tokens or not.
    #[test]
    fn a_token_added_by_an_update_to_a_contract_without_tokens_is_found() {
        let mut net = Net::new(7, &[2]);
        let mut cache = Cache::default();
        cache.begin();
        run(&mut cache, &mut net, 100);
        assert_eq!(found(&cache), ["T2"]);
        net.contracts.insert(id(4), (2, Some("T4".into())));
        cache.begin();
        run(&mut cache, &mut net, 100);
        assert_eq!(found(&cache), ["T2", "T4"]);
        assert_eq!(cache.tokens[&id(4)].version, 2);
    }

    #[test]
    fn an_answer_to_another_question_is_not_taken() {
        let mut net = Net::new(7, &[2]);
        let mut cache = Cache::default();
        cache.begin();
        let before = cache.clone();
        let wrong = Ask::Contracts(vec![id(2)]);
        let reply = net.answer(&wrong);
        assert_eq!(cache.take(&wrong, reply, LIMITS), Err(NotAsked));
        let asked = cache.ask(LIMITS).expect("ids");
        assert_eq!(
            cache.take(&asked, Reply::Versions(Vec::new()), LIMITS),
            Err(NotAsked),
            "a reply of another kind"
        );
        assert_eq!(cache, before);
        // A contract the network did not ask about is not taken either.
        run(&mut cache, &mut net, 3);
        if let Some(ask @ Ask::Contracts(_)) = cache.ask(LIMITS) {
            let stranger = Reply::Contracts(vec![Read {
                id: id(99),
                version: 1,
                tokens: Some("X".to_string()),
            }]);
            let before = cache.clone();
            assert_eq!(cache.take(&ask, stranger, LIMITS), Err(NotAsked));
            assert_eq!(cache, before);
        } else {
            panic!("contracts to read after the ids");
        }
    }
}
