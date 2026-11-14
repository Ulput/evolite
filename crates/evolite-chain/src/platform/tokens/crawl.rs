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
//!
//! A first pass (nothing known yet) reads the pages with their contracts instead (dev2-3,
//! `ids_only: false`): the ids and the contracts in one proved answer, about half the requests.
//! There is nothing to ask the versions of, and nothing left to read after the pages.

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
    /// A first pass: reading the pages of contracts, with their bodies, after `after`.
    Pages { after: Option<Id> },
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
    /// A page of contracts with their bodies, after `after`.
    Pages {
        after: Option<Id>,
        limit: u32,
    },
    Ids {
        after: Option<Id>,
        limit: u32,
    },
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
    /// Every contract of the page, in id order.
    Pages(Vec<Read<P>>),
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
    /// Start a pass, or go on with the one cut short. With nothing known, the first pass reads
    /// pages with their contracts; later ones ids, then versions, then what changed.
    pub fn begin(&mut self) {
        if self.pass == Pass::Idle {
            self.pass = if self.ids.is_empty() {
                Pass::Pages { after: None }
            } else {
                Pass::Ids { after: None }
            };
        }
    }

    /// What to ask now; `None`: no pass under way (the last one is done).
    pub fn ask(&self, limits: Limits) -> Option<Ask> {
        match &self.pass {
            Pass::Idle => None,
            Pass::Pages { after } => Some(Ask::Pages {
                after: *after,
                limit: limits.ids_page,
            }),
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
            (Ask::Pages { after, limit }, Reply::Pages(page)) => {
                let after_asked = |r: &Read<P>| after.is_none_or(|a| r.id > a);
                if page.len() > *limit as usize || !page.iter().all(after_asked) {
                    return Err(NotAsked);
                }
                let last = page.iter().map(|r| r.id).max();
                let full = page.len() == *limit as usize;
                for r in page {
                    self.ids.insert(r.id, Some(r.version));
                    self.to_read.remove(&r.id);
                    self.keep(r);
                }
                // A short page is the last one; nothing is left to ask the versions of.
                self.pass = match last {
                    Some(last) if full => Pass::Pages { after: Some(last) },
                    _ => Pass::Contracts,
                };
            }
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
                    self.keep(r);
                }
            }
            _ => return Err(NotAsked),
        }
        self.settle();
        Ok(())
    }

    /// A contract read: kept among those with tokens, or taken out of them.
    fn keep(&mut self, r: Read<P>) {
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

        fn read(&self, i: &Id) -> Read<String> {
            let (version, tokens) = self.contracts[i].clone();
            Read {
                id: *i,
                version,
                tokens,
            }
        }

        fn answer(&mut self, ask: &Ask) -> Reply<String> {
            self.asked.push(ask.clone());
            match ask {
                Ask::Pages { after, limit } => Reply::Pages(
                    self.contracts
                        .keys()
                        .filter(|k| after.is_none_or(|a| **k > a))
                        .take(*limit as usize)
                        .map(|k| self.read(k))
                        .collect(),
                ),
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
                Ask::Contracts(ids) => Reply::Contracts(ids.iter().map(|i| self.read(i)).collect()),
            }
        }

        /// What was asked, by kind: pages, ids, versions, contracts.
        fn kinds(&self) -> [usize; 4] {
            let mut n = [0; 4];
            for a in &self.asked {
                n[match a {
                    Ask::Pages { .. } => 0,
                    Ask::Ids { .. } => 1,
                    Ask::Versions(_) => 2,
                    Ask::Contracts(_) => 3,
                }] += 1;
            }
            n
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

    /// dev2-3: a first pass reads the pages of `getDataContractsByRange` with their contracts, so
    /// the ids and the contracts come in one request: no list of ids, no versions, no batches.
    #[test]
    fn a_first_pass_reads_pages_with_their_contracts() {
        let mut net = Net::new(7, &[2, 5, 7]);
        let mut cache = Cache::default();
        assert_eq!(cache.ask(LIMITS), None, "nothing before a pass begins");
        cache.begin();
        run(&mut cache, &mut net, 100);
        assert_eq!(cache.pass, Pass::Idle);
        assert_eq!(cache.passes, 1);
        assert_eq!(cache.ids.len(), 7);
        assert!(cache.ids.values().all(|v| *v == Some(1)), "every one read");
        assert_eq!(found(&cache), ["T2", "T5", "T7"]);
        assert!(cache.to_read.is_empty());
        assert_eq!(net.kinds(), [3, 0, 0, 0], "three pages and nothing else");
        // Pages of 3 after the last id: 1-3, 4-6, 7 (a short page ends them).
        let pages: Vec<Option<Id>> = net
            .asked
            .iter()
            .filter_map(|a| match a {
                Ask::Pages { after, .. } => Some(*after),
                _ => None,
            })
            .collect();
        assert_eq!(pages, [None, Some(id(3)), Some(id(6))]);
    }

    /// A full last page is followed by an empty one, which ends the pages.
    #[test]
    fn an_empty_page_after_a_full_one_ends_the_pages() {
        let mut net = Net::new(6, &[6]);
        let mut cache = Cache::default();
        cache.begin();
        run(&mut cache, &mut net, 100);
        assert_eq!(cache.passes, 1);
        assert_eq!(found(&cache), ["T6"]);
        assert_eq!(net.kinds(), [3, 0, 0, 0]);
    }

    /// Architect: a first pass cut short among the pages (a lock, no network) goes on from the
    /// page after the last one taken; the pages taken are not read again, nothing is lost.
    #[test]
    fn a_first_pass_cut_among_the_pages_goes_on_from_the_next_page() {
        let mut net = Net::new(8, &[2, 5, 7]);
        let mut cache = Cache::default();
        cache.begin();
        run(&mut cache, &mut net, 1);
        assert_eq!(cache.pass, Pass::Pages { after: Some(id(3)) });
        assert_eq!(found(&cache), ["T2"], "the first page's token is kept");
        let mut kept = cache.clone();
        kept.begin();
        assert_eq!(
            kept.ask(LIMITS),
            Some(Ask::Pages {
                after: Some(id(3)),
                limit: 3
            })
        );
        net.asked.clear();
        run(&mut kept, &mut net, 100);
        assert_eq!(found(&kept), ["T2", "T5", "T7"]);
        assert_eq!(kept.passes, 1);
        assert_eq!(net.kinds(), [2, 0, 0, 0], "pages 4-6 and 7-8 only");
        assert_eq!(kept.ids.len(), 8);
    }

    /// A page that is not the one asked (an id at or before the cursor, or more than the limit)
    /// is not taken.
    #[test]
    fn a_page_that_was_not_asked_is_not_taken() {
        let mut net = Net::new(7, &[2]);
        let mut cache = Cache::default();
        cache.begin();
        run(&mut cache, &mut net, 1);
        let ask = cache.ask(LIMITS).expect("the second page");
        let before = cache.clone();
        let behind = Reply::Pages(vec![net.read(&id(2))]);
        assert_eq!(cache.take(&ask, behind, LIMITS), Err(NotAsked));
        let too_long = Reply::Pages((4..=7).map(|i| net.read(&id(i))).collect());
        assert_eq!(cache.take(&ask, too_long, LIMITS), Err(NotAsked));
        assert_eq!(cache, before);
    }

    /// A cache an older app left among its ids goes on as it began: by ids, then contracts.
    #[test]
    fn a_pass_begun_by_ids_goes_on_by_ids() {
        let mut net = Net::new(7, &[2, 5, 7]);
        let mut cache: Cache<String> = Cache {
            ids: [(id(1), None), (id(2), None), (id(3), None)].into(),
            to_read: [id(1), id(2), id(3)].into(),
            pass: Pass::Ids { after: Some(id(3)) },
            ..Cache::default()
        };
        cache.begin();
        run(&mut cache, &mut net, 100);
        assert_eq!(found(&cache), ["T2", "T5", "T7"]);
        assert_eq!(cache.passes, 1);
        assert_eq!(net.kinds()[0], 0, "no pages");
    }

    #[test]
    fn a_pass_cut_short_goes_on_from_its_last_id() {
        // The second pass reads ids, then the new contracts.
        let mut net = Net::new(4, &[2]);
        let mut cache = Cache::default();
        cache.begin();
        run(&mut cache, &mut net, 100);
        for i in 5..=9 {
            net.contracts
                .insert(id(i), (1, (i % 2 == 1).then(|| format!("T{i}"))));
        }
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
        assert_eq!(found(&kept), ["T2", "T5", "T7", "T9"]);
        assert_eq!(kept.passes, 2);
        // Cut short among the contracts: the ones read are not read again.
        let mut cache = kept.clone();
        for i in 10..=14 {
            net.contracts.insert(id(i), (1, None));
        }
        cache.begin();
        loop {
            let ask = cache.ask(LIMITS).expect("a pass");
            let reply = net.answer(&ask);
            cache.take(&ask, reply, LIMITS).unwrap();
            if matches!(cache.ask(LIMITS), Some(Ask::Contracts(_))) {
                break;
            }
        }
        let ask = cache.ask(LIMITS).unwrap();
        let reply = net.answer(&ask);
        cache.take(&ask, reply, LIMITS).unwrap();
        assert_eq!(cache.pass, Pass::Contracts);
        let left = cache.to_read.len();
        assert!(left > 0);
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
        assert_eq!(kept.passes, 3);
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
        // A contract the network did not ask about is not taken either: a second pass, with
        // new contracts to read after the ids and the versions.
        run(&mut cache, &mut net, 100);
        net.contracts.insert(id(8), (1, None));
        cache.begin();
        while !matches!(cache.ask(LIMITS), Some(Ask::Contracts(_)) | None) {
            let ask = cache.ask(LIMITS).unwrap();
            let reply = net.answer(&ask);
            cache.take(&ask, reply, LIMITS).unwrap();
        }
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
