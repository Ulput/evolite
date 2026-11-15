//! The wallet's claims on contested names, kept up to date while the wallet is open (FEAT-0021,
//! АРХ-1, R-9): one proved read of each open request's vote, every 2 minutes on testnet and every
//! 15 on mainnet, and at once when woken. Nothing is read when no request is open, and a read that
//! fails leaves the request as it was (AC-T4). A result is told once, as
//! [`Event::NameContestEnded`] (R-11), and a win reads the identity's names again (AC-T7).

use crate::real::*;

/// How often open requests are read: the vote lasts 90 minutes on testnet, 14 days on mainnet.
pub(in crate::real) fn poll_every(mainnet: bool) -> Duration {
    Duration::from_secs(if mainnet { 15 * 60 } else { 2 * 60 })
}

/// The requests a round reads (AC-T4): the open ones only — none when none is open.
pub(in crate::real) fn to_read(requests: &[name_requests::NameRequest]) -> Vec<(String, String)> {
    requests
        .iter()
        .filter(|r| r.result.is_none())
        .map(|r| (r.identity.clone(), r.label.clone()))
        .collect()
}

/// What the poll keeps and tells: the requests' file, the events' way out, the journal for
/// History's result line (N6), the network, and what wakes it before its time.
pub(in crate::real) struct NamePoll {
    pub path: PathBuf,
    pub out: Sender<Event>,
    pub journal: Arc<journal::Journal>,
    pub mainnet: bool,
    pub wake: Arc<tokio::sync::Notify>,
}

/// History's line for how a vote ended (FEAT-0021 N6): no amount, the identity that claimed.
pub(in crate::real) fn result_entry(
    identity: &str,
    label: &str,
    result: NameState,
) -> journal::Entry {
    let outcome = match result {
        NameState::Won { .. } => "won",
        NameState::Locked { .. } => "locked",
        _ => "lost",
    };
    journal::Entry {
        at: now(),
        what: journal::What::NameResult {
            label: label.to_owned(),
            outcome: outcome.into(),
        },
        spent: 0,
        moved: 0,
        identity: Some(identity.to_owned()),
        ..Default::default()
    }
}

/// Reads the open requests of `wallet` while `live` holds; `poll.wake` reads them at once.
pub(in crate::real) async fn keep_name_requests_fresh(
    chain: Arc<Chain>,
    wallet_id: WalletId,
    live: impl Fn() -> bool,
    poll: NamePoll,
    refresh: impl Fn(),
) {
    let file = name_requests::NameRequests::new(poll.path);
    while live() {
        let _ = file.prune(now());
        for (identity, label) in to_read(&file.load()) {
            if !live() {
                return;
            }
            // A failed read keeps the request as it was (AC-T4).
            let Ok((contest, _)) = chain.contest(&label).await else {
                continue;
            };
            let mut ended = None;
            let _ = file.update(&identity, &label, |r| {
                ended = r.read(&contest, now()).map(|_| r.state(now()))
            });
            if let Some(result) = ended {
                if matches!(result, NameState::Won { .. })
                    && let Some(wallet) = chain.wallet(&wallet_id).await
                {
                    let _ = chain.sync_names(&wallet, &identity).await;
                }
                poll.journal.add(result_entry(&identity, &label, result));
                let _ = poll.out.send(Event::NameContestEnded {
                    identity: identity.clone(),
                    label: label.clone(),
                    result,
                });
            }
            refresh();
        }
        tokio::select! {
            _ = poll.wake.notified() => {}
            _ = tokio::time::sleep(poll_every(poll.mainnet)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use name_requests::{NameRequest, Outcome, Tally};

    /// AC-T4: the interval follows the network, and a round reads the open requests only.
    #[test]
    fn open_requests_alone_are_read_at_the_networks_pace() {
        assert_eq!(poll_every(false), Duration::from_secs(120));
        assert_eq!(poll_every(true), Duration::from_secs(900));
        let request = |label: &str, result| NameRequest {
            identity: "A".into(),
            label: label.into(),
            sent_at: 0,
            join_until: 0,
            ends_about: 0,
            tally: Tally::default(),
            fund: 0,
            result,
            hidden: false,
            contenders: Vec::new(),
            winner: None,
            checked_at: None,
            notice_closed: false,
            result_notice_closed: false,
            link: None,
            link_written: false,
        };
        assert!(to_read(&[]).is_empty());
        assert!(to_read(&[request("won", Some((Outcome::Won, 1)))]).is_empty());
        assert_eq!(
            to_read(&[
                request("lost", Some((Outcome::Lost, 1))),
                request("sam", None)
            ]),
            vec![("A".to_string(), "sam".to_string())]
        );
    }

    /// N6, AC-T19: a vote's end writes History a line of its own — no amount, the claiming
    /// identity, how it ended — beside the claim's line, which it does not change.
    #[test]
    fn a_votes_end_is_a_history_line_of_its_own() {
        for (state, outcome) in [
            (NameState::Won { at: 1 }, "won"),
            (NameState::Lost { at: 1 }, "lost"),
            (NameState::Locked { at: 1 }, "locked"),
        ] {
            let entry = result_entry("A", "sam", state);
            assert_eq!(
                entry.what,
                journal::What::NameResult {
                    label: "sam".into(),
                    outcome: outcome.into()
                }
            );
            assert_eq!((entry.spent, entry.moved), (0, 0));
            assert_eq!(entry.identity.as_deref(), Some("A"));
        }
    }
}
