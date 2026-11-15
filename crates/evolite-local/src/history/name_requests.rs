//! The wallet's claims on contested DPNS names (FEAT-0021, АРХ-1 and АРХ-5): which identity asked
//! for which label, when, when its masternode vote should end, the last tally read and, once the
//! network has told it, the result. The vote's answer has no end time, and platform-wallet lists a
//! contested name as soon as it is sent, so this file is what says a name is still in the vote.
//! Public data only (identity ids, labels, times, vote counts). Also the pure rules over it: what a
//! name check answers (AC-T1), where a request stands by the clock (AC-T2), and whether a lost or
//! blocked name is still shown (R-12, AC-T17).

use evolite_api::{NameCheck, NameState, Timestamp};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How long a lost or blocked name stays among the identity's names after its result (R-12).
pub const SHOWN_AFTER_RESULT: i64 = 30 * 86_400;
/// How long the request itself is kept after its result: the contest card opened from History
/// reads it (R-12).
pub const KEPT_AFTER_RESULT: i64 = 90 * 86_400;

/// How a vote ended, as the network proved it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Outcome {
    Won,
    Lost,
    Locked,
}

/// The last tally read for a request.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Tally {
    pub contenders: u32,
    pub ours: u32,
    pub lock: u32,
    pub abstain: u32,
}

/// One claim. Times are Unix seconds on this computer's clock.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct NameRequest {
    /// The identity that asked, in base58.
    pub identity: String,
    /// The label as the person typed it (shown); [`evolite_common::dpns::normalized`] of it is
    /// what the network compares.
    pub label: String,
    pub sent_at: Timestamp,
    /// Until when others may still join, and the estimated end: from the contest's start (its
    /// first claim, which may be another identity's) and the protocol's durations.
    pub join_until: Timestamp,
    pub ends_about: Timestamp,
    #[serde(default)]
    pub tally: Tally,
    /// The fund paid, in duffs, for the History line and «the fund was not returned» (R-12).
    #[serde(default)]
    pub fund: i64,
    /// The result and when this computer first had it proved (`result_at`, R-12).
    #[serde(default)]
    pub result: Option<(Outcome, Timestamp)>,
    /// «Hide» was pressed (R-12): the row is no longer shown; nothing else changes.
    #[serde(default)]
    pub hidden: bool,
    /// Every claim and its votes, the winner, and when they were read with proof (N5).
    #[serde(default)]
    pub contenders: Vec<(String, u32)>,
    #[serde(default)]
    pub winner: Option<String>,
    #[serde(default)]
    pub checked_at: Option<Timestamp>,
    /// Overview's notice closed with «×»: of the vote, and of its result (review r1, Should 1).
    #[serde(default)]
    pub notice_closed: bool,
    #[serde(default)]
    pub result_notice_closed: bool,
    /// The link «where my identity is confirmed» (АРХ-6) and whether it is written (AC-T12).
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub link_written: bool,
}

impl NameRequest {
    /// What the contest card shows of it (N5).
    pub fn contest(&self, now: Timestamp) -> evolite_api::NameContest {
        evolite_api::NameContest {
            contenders: self.contenders.clone(),
            winner: self.winner.clone(),
            fund: self.fund,
            sent_at: self.sent_at,
            checked_at: self.checked_at,
            listed: self.shown(now),
            notice_closed: self.notice_closed,
            result_notice_closed: self.result_notice_closed,
            link: self.link.clone(),
            link_written: self.link_written,
        }
    }

    /// «×» on Overview's notice (Should 1): the vote's while it runs, the result's once there is
    /// one — the result is another notice and shows anew.
    pub fn close_notice(&mut self) {
        if self.result.is_some() {
            self.result_notice_closed = true;
        } else {
            self.notice_closed = true;
        }
    }

    fn take_claims(&mut self, contest: &ContestSnapshot, now: Timestamp) {
        self.contenders = contest
            .contenders
            .iter()
            .map(|c| (c.identity.clone(), c.votes))
            .collect();
        self.winner = match &contest.winner {
            Some(Winner::Identity(id)) => Some(id.clone()),
            _ => None,
        };
        self.checked_at = Some(now);
    }
    /// Where it stands at `now` (AC-T2): the result once the network told it; before that, in the
    /// vote until its estimated end, then waiting for the result. The clock never makes a result.
    pub fn state(&self, now: Timestamp) -> NameState {
        match self.result {
            Some((Outcome::Won, at)) => NameState::Won { at },
            Some((Outcome::Lost, at)) => NameState::Lost { at },
            Some((Outcome::Locked, at)) => NameState::Locked { at },
            None if now >= self.ends_about => NameState::AwaitingResult {
                ends_about: self.ends_about,
            },
            None => NameState::Voting {
                join_until: self.join_until,
                ends_about: self.ends_about,
                contenders: self.tally.contenders,
                ours: self.tally.ours,
                lock: self.tally.lock,
                abstain: self.tally.abstain,
            },
        }
    }

    /// Whether the identity's names show it at `now` (AC-T17): a lost or blocked one until 30 days
    /// after its result, that day included, unless hidden; any other always. A clock set back
    /// does not bring a hidden one back.
    pub fn shown(&self, now: Timestamp) -> bool {
        match self.result {
            Some((Outcome::Lost | Outcome::Locked, at)) => {
                !self.hidden && now - at <= SHOWN_AFTER_RESULT
            }
            _ => true,
        }
    }

    /// Takes a proved vote state read at `now` (AC-T4, AC-T7): the tally, and the result once the
    /// network has one — said back only the first time (`result_at` is then `now`, R-12). A
    /// request that has its result does not change any more.
    pub fn read(&mut self, contest: &ContestSnapshot, now: Timestamp) -> Option<Outcome> {
        if self.result.is_some() {
            return None;
        }
        self.tally = contest.tally_for(&self.identity);
        self.take_claims(contest, now);
        let outcome = contest.outcome_for(&self.identity)?;
        self.result = Some((outcome, now));
        Some(outcome)
    }

    /// Still worth keeping at `now`: until 90 days after its result.
    fn kept(&self, now: Timestamp) -> bool {
        self.result
            .is_none_or(|(_, at)| now - at <= KEPT_AFTER_RESULT)
    }

    fn is(&self, identity: &str, label: &str) -> bool {
        self.identity == identity && same_name(&self.label, label)
    }
}

/// One name to DPNS: the same in lower case and its homograph-safe form (R-6).
pub fn same_name(a: &str, b: &str) -> bool {
    let form = |l: &str| evolite_common::dpns::normalized(&l.to_lowercase());
    form(a) == form(b)
}

/// Who got a finished contest.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Winner {
    Identity(String),
    Locked,
    NoWinner,
}

/// One claim in a contest: its identity (base58), its votes, and when its document was made.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Contender {
    pub identity: String,
    pub votes: u32,
    pub created_at: Option<Timestamp>,
}

/// A contest's vote state as the network proved it, without the SDK's types.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ContestSnapshot {
    pub winner: Option<Winner>,
    pub contenders: Vec<Contender>,
    pub lock: u32,
    pub abstain: u32,
}

impl ContestSnapshot {
    /// When the contest started: its earliest claim.
    pub fn started_at(&self) -> Option<Timestamp> {
        self.contenders.iter().filter_map(|c| c.created_at).min()
    }

    /// The result for `identity`'s claim, if the contest has one. «No winner» happens only when no
    /// claim is left (АРХ-2 correction 2): for a claim of ours that is not a win.
    pub fn outcome_for(&self, identity: &str) -> Option<Outcome> {
        Some(match self.winner.as_ref()? {
            Winner::Identity(id) if id == identity => Outcome::Won,
            Winner::Identity(_) | Winner::NoWinner => Outcome::Lost,
            Winner::Locked => Outcome::Locked,
        })
    }

    /// The tally as `identity` sees it.
    pub fn tally_for(&self, identity: &str) -> Tally {
        Tally {
            contenders: self.contenders.len() as u32,
            ours: self
                .contenders
                .iter()
                .find(|c| c.identity == identity)
                .map_or(0, |c| c.votes),
            lock: self.lock,
            abstain: self.abstain,
        }
    }
}

/// What a check of `label` answers (AC-T1, as DET's `classify_availability`): `awarded` — a domain
/// with this label exists; `contest` — its proved vote state, read for a contested label only;
/// `shown` — the identity on screen; `wallet` — every identity of this wallet with its slot;
/// `(join_secs, vote_secs)` — the protocol's window for joining and the vote's length.
pub fn classify(
    label: &str,
    awarded: bool,
    contest: Option<&ContestSnapshot>,
    shown: &str,
    wallet: &[(String, u32)],
    now: Timestamp,
    (join_secs, vote_secs): (u64, u64),
) -> NameCheck {
    if awarded {
        return NameCheck::Taken;
    }
    if let Some(contest) = contest {
        if contest.winner.is_none()
            && let Some(ours) = contest
                .contenders
                .iter()
                .find_map(|c| wallet.iter().find(|(id, _)| *id == c.identity))
        {
            return NameCheck::AlreadyRequested {
                other: (ours.0 != shown).then_some(ours.1),
            };
        }
        match contest.winner {
            Some(Winner::Locked) => return NameCheck::Locked,
            Some(Winner::Identity(_)) => return NameCheck::Taken,
            Some(Winner::NoWinner) => {}
            None if !contest.contenders.is_empty() => {
                // Without a start time the window cannot be shown open.
                let start = contest.started_at();
                return match start {
                    Some(start) if now < start + join_secs as i64 => NameCheck::Joinable {
                        contenders: contest.contenders.len() as u32,
                        join_until: start + join_secs as i64,
                    },
                    _ => NameCheck::JoinClosed {
                        ends_about: start.map(|s| s + vote_secs as i64),
                    },
                };
            }
            None => {}
        }
    }
    if evolite_common::dpns::is_contested(label) {
        NameCheck::NeedsVote
    } else {
        NameCheck::Available
    }
}

/// Whether a claim on a contested name may be sent (FEAT-0021): only one the check finds
/// claimable, and only for a fund no higher than the one agreed to in the review (R-2, BUG-108:
/// what is promised is never less than what is charged). Nothing is sent otherwise.
pub fn judge_claim(
    check: NameCheck,
    fund_now: evolite_api::Duffs,
    agreed: Option<evolite_api::Duffs>,
) -> Result<(), evolite_api::CoreError> {
    if !check.claimable() {
        return Err(evolite_api::CoreError::NameUnavailable(check));
    }
    if check.contested() && agreed.is_none_or(|agreed| agreed < fund_now) {
        return Err(evolite_api::CoreError::ContestFundChanged { fund: fund_now });
    }
    Ok(())
}

impl NameRequest {
    /// The request kept once `identity`'s claim on `label` is in (FEAT-0021): its window and
    /// estimated end run from the contest's start, its first claim, which is ours when we opened
    /// it (`contest` then has no claim but ours, or none yet).
    pub fn new(
        identity: &str,
        label: &str,
        contest: &ContestSnapshot,
        fund: evolite_api::Duffs,
        now: Timestamp,
        join_secs: u64,
        vote_secs: u64,
    ) -> Self {
        let start = contest.started_at().unwrap_or(now).min(now);
        let mut tally = contest.tally_for(identity);
        if !contest.contenders.iter().any(|c| c.identity == identity) {
            tally.contenders += 1;
        }
        let mut request = Self {
            identity: identity.into(),
            label: label.into(),
            sent_at: now,
            join_until: start + join_secs as i64,
            ends_about: start + vote_secs as i64,
            tally,
            fund,
            result: None,
            hidden: false,
            contenders: Vec::new(),
            winner: None,
            checked_at: None,
            notice_closed: false,
            result_notice_closed: false,
            link: None,
            link_written: false,
        };
        request.take_claims(contest, now);
        if !request.contenders.iter().any(|(id, _)| id == identity) {
            request.contenders.push((identity.into(), 0));
        }
        request
    }
}

/// Lays the wallet's requests over `identity`'s names as listed (FEAT-0021, AC-T3): a request's
/// state wins over the list, which platform-wallet fills with a contested name the moment it is
/// sent; a request not in the list joins it; a lost or blocked one hidden or past its 30 days
/// stays, not listed. The primary name is the first one the identity owns.
pub fn apply(
    names: &mut Vec<evolite_api::DpnsName>,
    identity: &str,
    requests: &[NameRequest],
    now: Timestamp,
) {
    for request in requests.iter().filter(|r| r.identity == identity) {
        let at = names
            .iter()
            .position(|n| same_name(&n.label, &request.label));
        // A hidden or expired one stays, not listed: History still opens its card (AC-T19).
        match at {
            Some(i) => {
                names[i].state = request.state(now);
                names[i].contest = Some(request.contest(now));
            }
            None => names.push(evolite_api::DpnsName {
                label: request.label.clone(),
                primary: false,
                state: request.state(now),
                contest: Some(request.contest(now)),
            }),
        }
    }
    let primary = names.iter().position(|n| n.state.owned());
    for (i, name) in names.iter_mut().enumerate() {
        name.primary = Some(i) == primary;
    }
}

/// The file, in the wallet's folder.
pub struct NameRequests {
    path: PathBuf,
}

impl NameRequests {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Every request kept. A missing or unreadable file is none.
    pub fn load(&self) -> Vec<NameRequest> {
        crate::json_file::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Adds `request`, or replaces the one of its identity and label.
    pub fn put(&self, request: NameRequest) -> Result<(), String> {
        let mut all = self.load();
        all.retain(|r| !r.is(&request.identity, &request.label));
        all.push(request);
        self.save(&all)
    }

    /// Changes the request of `identity` and `label` with `change`; false when there is none.
    pub fn update(
        &self,
        identity: &str,
        label: &str,
        change: impl FnOnce(&mut NameRequest),
    ) -> Result<bool, String> {
        let mut all = self.load();
        let Some(request) = all.iter_mut().find(|r| r.is(identity, label)) else {
            return Ok(false);
        };
        let before = request.clone();
        change(request);
        if *request != before {
            self.save(&all)?;
        }
        Ok(true)
    }

    /// «Hide» (R-12, AC-T18): only a lost or blocked request of this identity; again is no error.
    /// False, and nothing written, when there is no such request.
    pub fn hide(&self, identity: &str, label: &str) -> Result<bool, String> {
        let all = self.load();
        let finished = all.iter().any(|r| {
            r.is(identity, label) && matches!(r.result, Some((Outcome::Lost | Outcome::Locked, _)))
        });
        if !finished {
            return Ok(false);
        }
        self.update(identity, label, |r| r.hidden = true)
    }

    /// Drops what is no longer kept at `now` (90 days after a result).
    pub fn prune(&self, now: Timestamp) -> Result<(), String> {
        let mut all = self.load();
        let before = all.len();
        all.retain(|r| r.kept(now));
        if all.len() != before {
            self.save(&all)?;
        }
        Ok(())
    }

    fn save(&self, all: &[NameRequest]) -> Result<(), String> {
        crate::json_file::save(&self.path, &all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn request(result: Option<(Outcome, Timestamp)>) -> NameRequest {
        NameRequest {
            identity: "A".into(),
            label: "sam".into(),
            sent_at: 1_000,
            join_until: 1_000 + 45 * 60,
            ends_about: 1_000 + 90 * 60,
            tally: Tally {
                contenders: 2,
                ours: 3,
                lock: 1,
                abstain: 0,
            },
            fund: 10_000_000,
            result,
            hidden: false,
            contenders: vec![("A".into(), 3)],
            winner: None,
            checked_at: None,
            notice_closed: false,
            result_notice_closed: false,
            link: None,
            link_written: false,
        }
    }

    fn contender(identity: &str, created_at: Timestamp) -> Contender {
        Contender {
            identity: identity.into(),
            votes: 0,
            created_at: Some(created_at),
        }
    }

    /// AC-T1: every one of the seven answers, from the domain and the vote state alone.
    #[test]
    fn a_name_check_answers_from_the_domain_and_the_vote_state() {
        let wallet = vec![("A".to_string(), 0), ("B".to_string(), 1)];
        let check = |label: &str, awarded, contest: Option<&ContestSnapshot>, now| {
            classify(
                label,
                awarded,
                contest,
                "A",
                &wallet,
                now,
                (45 * 60, 90 * 60),
            )
        };
        let open = ContestSnapshot {
            contenders: vec![contender("X", 1_000), contender("Y", 1_500)],
            ..Default::default()
        };
        assert_eq!(check("alice2", false, None, 0), NameCheck::Available);
        assert_eq!(check("sam", false, None, 0), NameCheck::NeedsVote);
        assert_eq!(check("sam", true, None, 0), NameCheck::Taken);
        assert_eq!(
            check("sam", false, Some(&open), 1_000 + 44 * 60),
            NameCheck::Joinable {
                contenders: 2,
                join_until: 1_000 + 45 * 60
            },
            "the window runs from the earliest claim"
        );
        assert_eq!(
            check("sam", false, Some(&open), 1_000 + 45 * 60),
            NameCheck::JoinClosed {
                ends_about: Some(1_000 + 90 * 60)
            }
        );
        let locked = ContestSnapshot {
            winner: Some(Winner::Locked),
            ..open.clone()
        };
        assert_eq!(check("sam", false, Some(&locked), 0), NameCheck::Locked);
        let won = ContestSnapshot {
            winner: Some(Winner::Identity("X".into())),
            ..open.clone()
        };
        assert_eq!(check("sam", false, Some(&won), 0), NameCheck::Taken);
        let nobody = ContestSnapshot {
            winner: Some(Winner::NoWinner),
            contenders: vec![],
            ..Default::default()
        };
        assert_eq!(check("sam", false, Some(&nobody), 0), NameCheck::NeedsVote);
        let mut ours = open.clone();
        ours.contenders.push(contender("A", 2_000));
        assert_eq!(
            check("sam", false, Some(&ours), 0),
            NameCheck::AlreadyRequested { other: None }
        );
        ours.contenders.pop();
        ours.contenders.push(contender("B", 2_000));
        assert_eq!(
            check("sam", false, Some(&ours), 0),
            NameCheck::AlreadyRequested { other: Some(1) },
            "R-7: another identity of this wallet"
        );
        let undated = ContestSnapshot {
            contenders: vec![Contender {
                identity: "X".into(),
                votes: 0,
                created_at: None,
            }],
            ..Default::default()
        };
        assert_eq!(
            check("sam", false, Some(&undated), 0),
            NameCheck::JoinClosed { ends_about: None },
            "no start time: never shown open"
        );
    }

    /// AC-T2: in the vote, then waiting by the clock; a result only from the network.
    #[test]
    fn a_request_moves_by_the_clock_but_its_result_comes_only_from_the_network() {
        let r = request(None);
        assert!(matches!(
            r.state(1_000),
            NameState::Voting {
                contenders: 2,
                ours: 3,
                lock: 1,
                ..
            }
        ));
        assert_eq!(
            r.state(1_000 + 90 * 60),
            NameState::AwaitingResult {
                ends_about: 1_000 + 90 * 60
            }
        );
        assert_eq!(
            r.state(1_000 + 365 * DAY),
            NameState::AwaitingResult {
                ends_about: 1_000 + 90 * 60
            },
            "a year later still no result made up"
        );
        assert_eq!(
            request(Some((Outcome::Lost, 9_000))).state(0),
            NameState::Lost { at: 9_000 }
        );
        let snapshot = ContestSnapshot {
            winner: Some(Winner::Identity("A".into())),
            contenders: vec![contender("A", 1_000)],
            ..Default::default()
        };
        assert_eq!(snapshot.outcome_for("A"), Some(Outcome::Won));
        assert_eq!(snapshot.outcome_for("B"), Some(Outcome::Lost));
        assert_eq!(ContestSnapshot::default().outcome_for("A"), None);
    }

    /// AC-T17: lost and blocked rows until 30 days after the result, that day included; hidden
    /// ones never; the others always; a clock set back brings no hidden row back.
    #[test]
    fn a_lost_or_blocked_name_is_shown_for_thirty_days_unless_hidden() {
        let at = 100 * DAY;
        for outcome in [Outcome::Lost, Outcome::Locked] {
            let r = request(Some((outcome, at)));
            assert!(r.shown(at));
            assert!(r.shown(at + 30 * DAY), "{outcome:?}: the 30th day included");
            assert!(!r.shown(at + 30 * DAY + 1), "{outcome:?}: then gone");
            let hidden = NameRequest {
                hidden: true,
                ..r.clone()
            };
            assert!(!hidden.shown(at), "{outcome:?}: hidden at once");
            assert!(
                !hidden.shown(at - 10 * DAY),
                "{outcome:?}: the clock set back"
            );
        }
        assert!(request(Some((Outcome::Won, at))).shown(at + 400 * DAY));
        assert!(
            request(None).shown(at + 400 * DAY),
            "in the vote or waiting: always"
        );
    }

    /// AC-T18 (the file's part): «Hide» is written and read back; only a lost or blocked request of
    /// that identity; again is no error; nothing is written for another identity or label.
    #[test]
    fn hide_is_kept_across_loads_and_only_for_a_finished_request_of_that_identity() {
        let dir =
            std::env::temp_dir().join(format!("evolite-name-requests-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("name-requests.json");
        let file = NameRequests::new(path.clone());
        assert!(file.load().is_empty());
        file.put(request(Some((Outcome::Lost, 5_000)))).unwrap();
        let mut voting = request(None);
        voting.label = "bob".into();
        file.put(voting).unwrap();
        let written = std::fs::read(&path).unwrap();

        assert_eq!(file.hide("B", "sam"), Ok(false), "another identity");
        assert_eq!(file.hide("A", "nobody"), Ok(false), "no such label");
        assert_eq!(file.hide("A", "bob"), Ok(false), "still in the vote");
        assert_eq!(std::fs::read(&path).unwrap(), written, "nothing written");

        assert_eq!(
            file.hide("A", "SAM"),
            Ok(true),
            "the label as the network compares it"
        );
        assert_eq!(file.hide("A", "sam"), Ok(true), "again: no error");
        let again = NameRequests::new(path);
        let sam = again.load().into_iter().find(|r| r.label == "sam").unwrap();
        assert!(sam.hidden && !sam.shown(5_000));

        again.prune(5_000 + KEPT_AFTER_RESULT + 1).unwrap();
        let left: Vec<String> = again.load().into_iter().map(|r| r.label).collect();
        assert_eq!(left, vec!["bob".to_string()], "kept 90 days after a result");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC-T3: a contested name platform-wallet already lists after it was sent is shown in the
    /// vote, not as got; one it does not list yet joins; a hidden lost one leaves; the primary
    /// name is never one still in the vote.
    #[test]
    fn a_sent_contested_name_is_never_shown_as_got() {
        use evolite_api::DpnsName;
        let listed = |label: &str| DpnsName {
            label: label.into(),
            primary: false,
            state: NameState::Registered,
            contest: None,
        };
        // platform-wallet's order: the contested one first, as it put it there when sent.
        let mut names = vec![listed("Sam"), listed("alice2")];
        let mut lost = request(Some((Outcome::Lost, 500)));
        lost.label = "bob".into();
        lost.hidden = true;
        let mut won_elsewhere = request(None);
        won_elsewhere.identity = "B".into();
        won_elsewhere.label = "alice2".into();
        let mut waiting = request(None);
        waiting.label = "carl".into();
        let mut names_with_lost = names.clone();
        names_with_lost.push(listed("bob"));
        names = names_with_lost;

        apply(
            &mut names,
            "A",
            &[request(None), lost, won_elsewhere, waiting],
            1_000,
        );

        let state = |label: &str| names.iter().find(|n| n.label == label).map(|n| n.state);
        assert!(
            matches!(state("Sam"), Some(NameState::Voting { .. })),
            "{names:?}"
        );
        assert_eq!(
            state("alice2"),
            Some(NameState::Registered),
            "another identity's request"
        );
        assert!(
            matches!(state("carl"), Some(NameState::Voting { .. })),
            "not listed yet"
        );
        let bob = names.iter().find(|n| n.label == "bob").unwrap();
        assert!(!bob.listed(), "hidden: kept for History, not listed");
        let primary: Vec<&str> = names
            .iter()
            .filter(|n| n.primary)
            .map(|n| n.label.as_str())
            .collect();
        assert_eq!(primary, vec!["alice2"], "the first name owned");
    }

    /// R-2 and R-3's part here: a claim goes only on a claimable name and only for a fund no higher
    /// than the one agreed to; a regular name needs no fund.
    #[test]
    fn a_claim_is_sent_only_claimable_and_for_the_fund_agreed() {
        use evolite_api::CoreError;
        let fund = 10_000_000;
        for refused in [
            NameCheck::Taken,
            NameCheck::Locked,
            NameCheck::JoinClosed { ends_about: None },
            NameCheck::AlreadyRequested { other: Some(1) },
        ] {
            assert_eq!(
                judge_claim(refused, fund, Some(fund)),
                Err(CoreError::NameUnavailable(refused))
            );
        }
        assert_eq!(judge_claim(NameCheck::NeedsVote, fund, Some(fund)), Ok(()));
        assert_eq!(
            judge_claim(NameCheck::NeedsVote, 2 * fund, Some(fund)),
            Err(CoreError::ContestFundChanged { fund: 2 * fund }),
            "the fund doubled since the review"
        );
        assert_eq!(
            judge_claim(NameCheck::NeedsVote, fund, None),
            Err(CoreError::ContestFundChanged { fund }),
            "never sent without a fund agreed"
        );
        assert_eq!(judge_claim(NameCheck::Available, fund, None), Ok(()));
    }

    /// A new request's window and end run from the contest's start: now when it is ours alone,
    /// the earliest claim when we joined one.
    #[test]
    fn a_new_request_runs_from_the_contests_start() {
        let fresh = NameRequest::new(
            "A",
            "sam",
            &ContestSnapshot::default(),
            7,
            5_000,
            2_700,
            5_400,
        );
        assert_eq!((fresh.join_until, fresh.ends_about), (7_700, 10_400));
        assert_eq!(fresh.tally.contenders, 1);
        let joined = ContestSnapshot {
            contenders: vec![contender("X", 4_000)],
            ..Default::default()
        };
        let r = NameRequest::new("A", "sam", &joined, 7, 5_000, 2_700, 5_400);
        assert_eq!((r.join_until, r.ends_about), (6_700, 9_400));
        assert_eq!(r.tally.contenders, 2);
    }

    /// AC-T4 and AC-T7 (the request's part): a read moves the tally; the result is taken from the
    /// network once, with when it came, and is said back that once; later reads change nothing.
    #[test]
    fn a_read_takes_the_tally_and_the_result_once() {
        let mut r = request(None);
        let running = ContestSnapshot {
            contenders: vec![
                Contender {
                    identity: "A".into(),
                    votes: 9,
                    created_at: Some(1_000),
                },
                contender("X", 1_200),
            ],
            lock: 4,
            ..Default::default()
        };
        assert_eq!(r.read(&running, 2_000), None);
        assert_eq!((r.tally.ours, r.tally.lock, r.tally.contenders), (9, 4, 2));
        assert!(r.result.is_none());
        let won = ContestSnapshot {
            winner: Some(Winner::Identity("A".into())),
            ..running.clone()
        };
        assert_eq!(r.read(&won, 7_000), Some(Outcome::Won));
        assert_eq!(r.result, Some((Outcome::Won, 7_000)));
        let after = r.clone();
        assert_eq!(r.read(&won, 9_000), None, "said once");
        assert_eq!(r, after, "a finished request no longer changes");
    }

    /// Review r1, Should 1: «×» on the vote's notice is kept in the file, so the notice stays
    /// closed after a restart until the result; the result's notice is another one, shown anew,
    /// and its «×» is its own.
    #[test]
    fn a_closed_notice_stays_closed_until_the_result() {
        let dir = std::env::temp_dir().join(format!("evolite-name-notice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("name-requests.json");
        let file = NameRequests::new(path.clone());
        file.put(request(None)).unwrap();
        file.update("A", "sam", NameRequest::close_notice).unwrap();
        let again = NameRequests::new(path.clone()).load();
        let c = again[0].contest(1_000);
        assert!(
            c.notice_closed && !c.result_notice_closed,
            "after a restart"
        );
        let result = ContestSnapshot {
            winner: Some(Winner::Identity("B".into())),
            ..Default::default()
        };
        file.update("A", "sam", |r| {
            r.read(&result, 2_000);
        })
        .unwrap();
        let c = file.load()[0].contest(2_000);
        assert!(!c.result_notice_closed, "the result's notice shows");
        file.update("A", "sam", NameRequest::close_notice).unwrap();
        assert!(file.load()[0].contest(2_000).result_notice_closed);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
