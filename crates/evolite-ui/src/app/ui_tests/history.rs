//! History, Receive and the private pool (branch H: BUG-47, BUG-39 №1, 10, 12): what a person
//! sees there.

use super::*;
use crate::screens::Screen;
use evolite_api::{Activity, ActivityKind, Chain, Proof};

/// The id of the operation [`with_undated`] adds.
const UNDATED: u64 = 0x4747_4747;

/// Puts a note found in the pool — an operation without a time of its own, as the real core
/// gives it — into the wallet the app shows, and draws. Again for a few frames: a snapshot of
/// the demo core in between would take it away.
fn with_undated(kit: &mut Kit) {
    for _ in 0..3 {
        let wallet = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
            .expect("a wallet");
        if !wallet.activity.iter().any(|a| a.id == UNDATED) {
            wallet.activity.push(Activity {
                id: UNDATED,
                kind: ActivityKind::PoolReceived,
                chain: Chain::Platform,
                at: 0,
                amount: 1_000_000,
                proof: Proof::Proved,
                txid: None,
                fee: None,
                block: None,
                address: None,
            });
        }
        kit.run_steps(1);
    }
}

/// Whether some node's name or words contain `part`.
fn says(kit: &Kit, part: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.contains(part)) || n.value().is_some_and(|v| v.contains(part))
    })
    .next()
    .is_some()
}

/// BUG-47: a note found in the pool has no time. History puts it under «No date» with
/// «Platform · time unknown», its card says «time unknown» — 1970 is nowhere.
#[test]
fn an_operation_without_a_time_is_not_dated_1970() {
    let (_turn, mut kit) = demo_app("activity", "en");
    // The pool's operations only: a short list, all of it on screen.
    kit.state_mut().filter = 3;
    with_undated(&mut kit);
    assert!(has_text(&kit, "NO DATE"), "the day header");
    assert!(
        says(&kit, "Received privately · ") && says(&kit, " · Platform · time unknown"),
        "the row"
    );
    assert!(!says(&kit, "JANUARY") && !says(&kit, "1970"));
    crate::screens::open_card(&kit.ctx, UNDATED);
    with_undated(&mut kit);
    assert!(
        has_text(&kit, "Platform · time unknown"),
        "the card's header"
    );
    assert!(!says(&kit, "1970") && !says(&kit, "Jan"));
}

/// BUG-39 №1: «Copy txid» in an operation's card says «Copied» for a moment, then is itself
/// again.
#[test]
fn copy_txid_says_copied_for_a_moment() {
    let (_turn, mut kit) = demo_app("activity-card", "en");
    wait_for(&mut kit, "the card's txid", |kit| {
        kit.query_by_label("Copy txid").is_some()
    });
    button(&kit, "Copy txid").click();
    kit.run_steps(2);
    assert!(kit.query_by_label("Copied").is_some(), "«Copied» at once");
    assert!(kit.query_by_label("Copy txid").is_none());
    // egui's clock moves with the frames.
    wait_for(&mut kit, "«Copy txid» back", |kit| {
        kit.query_by_label("Copy txid").is_some()
    });
    assert!(kit.query_by_label("Copied").is_none());
}

/// BUG-39 №1: «Copy» of the private address says «Copied» too.
#[test]
fn copy_of_the_private_address_says_copied() {
    let (_turn, mut kit) = demo_app("pool-address", "en");
    wait_for(&mut kit, "the private address", |kit| {
        kit.query_by_label("Copy").is_some()
    });
    button(&kit, "Copy").click();
    kit.run_steps(2);
    assert!(kit.query_by_label("Copied").is_some());
}

/// BUG-39 №10: an amount typed in «Request an amount» (a wrong one too) is gone after the
/// person leaves «Receive» and comes back; while there, it stays.
#[test]
fn receive_forgets_the_requested_amount_on_leaving() {
    let (_turn, mut kit) = demo_app("receive", "en");
    kit.state_mut().receive.amount = "abc".into();
    kit.run_steps(2);
    assert!(
        has_text(&kit, "Enter an amount, e.g. 0.25"),
        "the mistake is said"
    );
    assert_eq!(
        kit.state().receive.amount,
        "abc",
        "kept while on the screen"
    );
    kit.state_mut().screen = Screen::Overview;
    kit.run_steps(2);
    kit.state_mut().screen = Screen::Receive;
    kit.run_steps(2);
    assert_eq!(kit.state().receive.amount, "");
    assert!(!has_text(&kit, "Enter an amount, e.g. 0.25"));
}

/// BUG-39 №12: with no operations in the pool its card says so instead of standing empty.
#[test]
fn an_empty_pool_says_it_has_no_operations() {
    let (_turn, mut kit) = demo_app("shielded", "en");
    assert!(!has_text(&kit, "No operations in the pool yet"));
    for _ in 0..3 {
        if let Some(wallet) = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
        {
            wallet.pool.operations.clear();
        }
        kit.run_steps(1);
    }
    assert!(has_text(&kit, "No operations in the pool yet"));
}

/// Puts `ops` into the wallet the app shows, as [`with_undated`] does.
fn with_ops(kit: &mut Kit, ops: &[Activity]) {
    for _ in 0..3 {
        let wallet = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
            .expect("a wallet");
        for op in ops {
            if !wallet.activity.iter().any(|a| a.id == op.id) {
                wallet.activity.insert(0, op.clone());
            }
        }
        kit.run_steps(1);
    }
}

fn platform_op(id: u64, kind: ActivityKind, amount: i64) -> Activity {
    Activity {
        id,
        kind,
        chain: Chain::Platform,
        at: chrono::Utc::now().timestamp() - 3_600,
        amount,
        proof: Proof::Proved,
        txid: None,
        fee: None,
        block: None,
        address: None,
    }
}

/// BUG-58 part 2: a contact request to us is a line of the History with how it stands and the
/// identity of ours it came to; its card opens Contacts, where it is answered. Ours that
/// answered theirs says so, and a withdrawal says how far it got.
#[test]
fn requests_to_us_and_withdrawals_say_how_they_stand() {
    use evolite_api::{RequestState, WithdrawalState};
    let (_turn, mut kit) = demo_app("activity", "en");
    // Identity operations: a short list, all of it on screen.
    kit.state_mut().filter = 4;
    let waiting = 0x5858_0001;
    with_ops(
        &mut kit,
        &[
            platform_op(
                waiting,
                ActivityKind::ContactRequestIn {
                    from: "@carol".into(),
                    ours: Some("@satoshi".into()),
                    state: RequestState::Waiting,
                },
                0,
            ),
            platform_op(
                0x5858_0002,
                ActivityKind::ContactRequestIn {
                    from: "@dave".into(),
                    ours: None,
                    state: RequestState::TheyAccepted,
                },
                0,
            ),
            platform_op(
                0x5858_0003,
                ActivityKind::ContactRequest {
                    to: "@erin".into(),
                    accepting: true,
                },
                -1_000,
            ),
            platform_op(
                0x5858_0004,
                ActivityKind::Withdrawal {
                    to: "yWithdrawalAddress1234567890".into(),
                    status: Some(WithdrawalState::Complete),
                },
                -5_000_000,
            ),
        ],
    );
    assert!(says(&kit, "Contact request from @carol"), "the request");
    assert!(
        says(&kit, " · for @satoshi · Waiting for an answer"),
        "who it came to, how it stands"
    );
    assert!(says(&kit, "Your request was accepted: @dave"));
    assert!(says(&kit, " · You are contacts now"));
    assert!(says(&kit, "You accepted the request from @erin"));
    assert!(says(&kit, " · Paid out"), "the withdrawal's status");
    crate::screens::open_card(&kit.ctx, waiting);
    with_ops(&mut kit, &[]);
    assert!(
        says(&kit, "Status: Waiting for an answer"),
        "the card's status"
    );
    button(&kit, "Open in Contacts").click();
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::Contacts);
}

/// BUG-58: «Backup» says what the phrase brings back and what stays on this computer only.
#[test]
fn backup_says_what_the_phrase_brings_back() {
    let (_turn, kit) = demo_app("backup", "en");
    assert!(has_text(&kit, "What the phrase brings back"));
    assert!(has_text(&kit, "Kept only on this computer"));
    assert!(has_text(
        &kit,
        "Payment notes, the address book and settings. The phrase does not bring them back."
    ));
}
