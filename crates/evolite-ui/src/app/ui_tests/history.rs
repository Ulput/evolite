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
                    ours: None,
                },
                -1_000,
            ),
            platform_op(
                0x5858_0005,
                ActivityKind::ContactRequest {
                    to: "@frank".into(),
                    accepting: false,
                    ours: Some("@satoshi".into()),
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
    // BUG-69: ours says which identity of ours it went from.
    assert!(says(&kit, "Contact request to @frank"));
    assert!(says(&kit, " · from @satoshi"), "who sent it");
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
    assert!(has_text(&kit, crate::screens::settings::KEPT_HERE));
    // The designer's corrected text (BUG-58.md «1a»): no longer «the history of operations».
    assert!(has_text(
        &kit,
        "Core money, credits and the private pool; identities and names; DashPay contacts and profile. evolite reads again from the network the history of Core payments, names, contact requests, the profile, withdrawals and the pool."
    ));
    assert!(has_text(&kit, crate::screens::settings::KEPT_HERE_TEXT));
}

/// BUG-58 «1a»: «Delete the wallet from the computer» says, before deleting, what stays on the
/// network and what is lost with this computer's copy. The designer (2026-11-06): the line on
/// what stays is the first one, instead of the old intro that said nearly the same; in «Forgot
/// the password?» it comes after the line about the password.
#[test]
fn deleting_says_what_comes_back_and_what_does_not() {
    const PASSWORD: &str = "The password cannot be recovered: it encrypts the phrase on this computer. Delete the wallet from here and restore it — or another one — from the phrase.";
    use crate::screens::settings::{KEPT_HERE, KEPT_HERE_TEXT, STAYS_ON_NETWORK};
    for (preset, lines) in [
        ("delete", &[STAYS_ON_NETWORK, KEPT_HERE, KEPT_HERE_TEXT][..]),
        (
            "forgot",
            &[PASSWORD, STAYS_ON_NETWORK, KEPT_HERE, KEPT_HERE_TEXT][..],
        ),
    ] {
        let (_turn, kit) = demo_app(preset, "ru");
        let mut above = f32::MIN;
        for key in lines {
            let shown = crate::i18n::t(key).into_owned();
            assert_ne!(shown, *key, "«{key}» is in Russian");
            let top = kit
                .query_all_by(|n| {
                    n.value().as_deref() == Some(shown.as_str())
                        || n.label().as_deref() == Some(shown.as_str())
                })
                .next()
                .unwrap_or_else(|| panic!("{preset}: «{shown}» in the dialog"))
                .rect()
                .top();
            assert!(top > above, "{preset}: «{shown}» below the line before it");
            above = top;
        }
        let old = "The money, identities and names stay on the network. Only the recovery phrase brings them back.";
        assert!(!has_text(&kit, old), "{preset}: the old intro is gone");
        assert!(
            !has_text(&kit, &crate::i18n::t(old)),
            "{preset}: the old intro is gone"
        );
    }
}

/// BUG-72: «Receive» scrolls, as the other screens do (BUG-48). In a low window «Earlier
/// addresses» goes on under the bottom edge; the wheel brings the card's end into the window.
#[test]
fn receive_scrolls_to_the_end_of_earlier_addresses() {
    const HIGH: f32 = 520.0;
    let (_turn, mut kit) = demo_app("receive", "en");
    kit.set_size(egui::vec2(1024.0, HIGH));
    kit.run_steps(3);
    assert!(has_text(&kit, "Earlier addresses"), "the demo has some");
    // The lowest thing drawn on the screen, right of the menu: the card's last row.
    let lowest = |kit: &Kit| {
        kit.query_all_by(|n| matches!(n.role(), Role::Button | Role::Label))
            .filter(|n| n.rect().left() > 300.0)
            .map(|n| n.rect().bottom())
            .fold(f32::MIN, f32::max)
    };
    let before = lowest(&kit);
    assert!(before > HIGH, "the card goes on under the edge: {before}");
    let title = words(&kit, "Earlier addresses").rect();
    assert!(
        title.top() > HIGH - 20.0,
        "the card starts at the edge: {title:?}"
    );
    // The pointer over the screen, right of the menu (the card itself is under the edge).
    kit.hover_at(egui::pos2(600.0, HIGH / 2.0));
    for _ in 0..4 {
        kit.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -400.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        kit.run_steps(4);
    }
    let after = lowest(&kit);
    let moved = words(&kit, "Earlier addresses").rect();
    assert!(
        moved.bottom() < HIGH,
        "the title came up: {title:?} → {moved:?}"
    );
    assert!(
        after <= HIGH,
        "the card's end is in the window after the wheel: {before} → {after}"
    );
}

/// The id of the asset lock [`lock`] makes.
const LOCK: u64 = 0x5757_0001;

/// An asset lock of 0.5 DASH to @werner555, its Core transaction chain-locked: a top-up or the
/// identity's creation, its credits as given.
fn lock(creates: bool, credits: evolite_api::Credits) -> Activity {
    Activity {
        id: LOCK,
        kind: ActivityKind::TopUp {
            identity: "@werner555".into(),
            creates,
            credits,
        },
        chain: Chain::CoreToPlatform,
        at: chrono::Utc::now().timestamp() - 3_600,
        amount: -50_000_263,
        proof: Proof::ChainLock,
        txid: Some("57".repeat(32)),
        fee: Some(263),
        block: Some(1_583_410),
        address: None,
    }
}

/// Puts `op` into the shown wallet in place of the line with its id, for a few frames (the
/// demo core's snapshots in between would take it away).
fn with_op(kit: &mut Kit, op: &Activity) {
    for _ in 0..3 {
        let wallet = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
            .expect("a wallet");
        wallet.activity.retain(|a| a.id != op.id);
        wallet.activity.insert(0, op.clone());
        kit.run_steps(1);
    }
}

/// BUG-57: a top-up Platform has not credited is not «final» and nothing more: its row says in
/// red that the credits did not come, its card says «Credits: not added», why, and leads to
/// Identity, where «Continue the top-up» is.
#[test]
fn a_top_up_without_credits_says_so_and_leads_to_identity() {
    use evolite_api::Credits;
    let (_turn, mut kit) = demo_app("activity", "en");
    kit.state_mut().filter = 4;
    with_op(&mut kit, &lock(false, Credits::NotAdded));
    assert!(says(&kit, "Top-up of @werner555 · "), "the row's title");
    assert!(says(&kit, " · credits not added"), "the row says it");
    crate::screens::open_card(&kit.ctx, LOCK);
    with_op(&mut kit, &lock(false, Credits::NotAdded));
    assert!(says(&kit, "Credits: not added"), "the card's line");
    assert!(has_text(&kit, "@werner555"), "to whom");
    assert!(has_text(&kit, "your identity"));
    assert!(has_text(
        &kit,
        "The top-up was interrupted. You can continue it on the Identity screen; the DASH are not lost"
    ));
    assert!(
        !says(&kit, "An identity or the Platform address of this wallet"),
        "no vague recipient"
    );
    button(&kit, "Continue on the Identity screen").click();
    kit.run_steps(3);
    assert_eq!(kit.state().screen, Screen::Identity);
    assert!(
        kit.query_by_label("Continue on the Identity screen")
            .is_none(),
        "the card closed"
    );
}

/// BUG-57: once «Continue» went through, the same line says so at once — no red, «added» in
/// the card, no way to continue — on History and on Overview's latest operations.
#[test]
fn a_top_up_that_finishes_stops_saying_not_added() {
    use evolite_api::Credits;
    let (_turn, mut kit) = demo_app("activity", "en");
    kit.state_mut().filter = 4;
    with_op(&mut kit, &lock(false, Credits::NotAdded));
    assert!(says(&kit, " · credits not added"));
    with_op(&mut kit, &lock(false, Credits::Added));
    assert!(says(&kit, "Top-up of @werner555 · "));
    assert!(!says(&kit, "credits not added"), "the row updated");
    crate::screens::open_card(&kit.ctx, LOCK);
    with_op(&mut kit, &lock(false, Credits::Added));
    assert!(says(&kit, "Credits: added"));
    assert!(
        kit.query_by_label("Continue on the Identity screen")
            .is_none()
    );
    assert!(!says(&kit, "The top-up was interrupted"));
    // Not known (a restore found the lock): no line about credits at all.
    with_op(&mut kit, &lock(false, Credits::Unknown));
    assert!(!says(&kit, "Credits: "), "nothing said when not known");
    // Overview's latest operations draw the same row.
    kit.ctx
        .data_mut(|d| d.remove::<u64>(egui::Id::new("activity-card")));
    kit.state_mut().screen = Screen::Overview;
    with_op(&mut kit, &lock(false, Credits::NotAdded));
    assert!(says(&kit, " · credits not added"), "Overview too");
    with_op(&mut kit, &lock(false, Credits::Added));
    assert!(!says(&kit, "credits not added"));
}

/// BUG-57: an asset lock that made the identity is its creation, not a top-up — in Russian
/// «Создание identity @…» — and one cut short says the creation is to be continued.
#[test]
fn an_identity_creation_is_not_called_a_top_up() {
    use evolite_api::Credits;
    let (_turn, mut kit) = demo_app("activity", "ru");
    kit.state_mut().filter = 4;
    with_op(&mut kit, &lock(true, Credits::Added));
    assert!(says(&kit, "Создание identity @werner555 · "), "the row");
    assert!(!says(&kit, "Пополнение @werner555"));
    with_op(&mut kit, &lock(true, Credits::NotAdded));
    assert!(says(&kit, " · кредиты не зачислены"));
    crate::screens::open_card(&kit.ctx, LOCK);
    with_op(&mut kit, &lock(true, Credits::NotAdded));
    assert!(says(&kit, "Кредиты: не зачислены"));
    assert!(has_text(
        &kit,
        "Создание identity прервалось. Его можно продолжить на экране Identity — DASH не потеряны"
    ));
    assert!(
        kit.query_by_label("Продолжить на экране Identity")
            .is_some()
    );
}
