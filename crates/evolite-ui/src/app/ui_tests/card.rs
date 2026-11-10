//! FEAT-0005a: the full card of a Core operation — from where to where, the parts of the
//! amount, the proof in words — on the demo's payment out, payment in and transfer between own
//! addresses; the other cards as they were.

use super::*;
use crate::format;
use crate::text::group_thousands;

/// The node whose words (name or label's value) are `text`, found once the card is up.
fn shown(kit: &mut Kit, text: &str) -> egui::Rect {
    wait_for(kit, text, |kit| has_text(kit, text));
    kit.query_all_by(|n| n.label().as_deref() == Some(text) || n.value().as_deref() == Some(text))
        .next()
        .expect("shown")
        .rect()
}

fn any_starts(kit: &Kit, start: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.starts_with(start))
            || n.value().is_some_and(|v| v.starts_with(start))
    })
    .next()
    .is_some()
}

/// Mockup 01, AC-A1/AC-T11: a transfer between own addresses says how much moved, from which
/// address of ours to which, the fee apart, and the ChainLock with its block — no «Where» and
/// no «Block» rows (the header and the proof's line say them).
#[test]
fn a_transfer_between_own_addresses_shows_both_addresses_and_the_fee() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    shown(
        &mut kit,
        "From: yNBw5qT8mK2vL6pR9xC3sD1fG7hZ4aE2iReb · this wallet's address",
    );
    shown(
        &mut kit,
        "To: yhR9mK3vT6pL2wQ8xN5cB1sD7fG4aZ9eo4nr · this wallet's address",
    );
    let moved = format::balance(1_000_000);
    shown(
        &mut kit,
        &format!("Moved {moved} inside the wallet: only the fee left it."),
    );
    shown(&mut kit, &format!("Moved: {moved}"));
    shown(&mut kit, &format!("Fee: {}", format::amount(226)));
    shown(
        &mut kit,
        &format!(
            "ChainLock in block {}: the quorum signature is checked",
            group_thousands(1_283_402)
        ),
    );
    assert!(!any_starts(&kit, "Where:"), "the chain is in the header");
    assert!(
        !any_starts(&kit, "Block:"),
        "the block is in the proof's line"
    );
    assert!(!any_starts(&kit, "Part of the details"));
}

/// Mockup 02: a payment out comes from «Core · this wallet» and its inputs (the change not
/// shown); the note sits under the proof's line and over the txid (FEAT-0005 item 9).
#[test]
fn a_payment_out_comes_from_this_wallet_and_its_note_sits_over_the_txid() {
    let (_turn, mut kit) = demo_app("activity-card", "en");
    let from = shown(&mut kit, "From: Core · this wallet · 3 inputs");
    // The payee as the person named it in the address book.
    let to = shown(
        &mut kit,
        "To: yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c · “Exchange · withdrawal” · from the address book",
    );
    assert!(from.top() < to.top());
    let proof = shown(
        &mut kit,
        &format!(
            "ChainLock in block {}: the quorum signature is checked",
            group_thousands(1_284_101)
        ),
    );
    let note = shown(
        &mut kit,
        "For the logo design · invoice no. 14 of 30 October",
    );
    let txid = kit.get_by_role_and_label(Role::Button, "Copy txid").rect();
    assert!(
        proof.bottom() <= note.top() && note.bottom() <= txid.top(),
        "proof {proof:?}, note {note:?}, txid {txid:?}"
    );
    assert!(!any_starts(&kit, "Where:") && !any_starts(&kit, "Block:"));
}

/// A payment in: its sender, and the address of ours it came to; in Russian too (the
/// designer's words).
#[test]
fn a_payment_in_names_the_sender_and_our_address() {
    let (_turn, mut kit) = demo_app("activity-card-received", "en");
    shown(&mut kit, "From: exchange");
    shown(
        &mut kit,
        "To: yNw3Hc6bV1qK8tR5mZ2xF9jD4sL7gP3aE6u · this wallet's address",
    );
    drop(kit);
    drop(_turn);
    let (_turn, mut kit) = demo_app("activity-card-received", "ru");
    shown(
        &mut kit,
        "Куда: yNw3Hc6bV1qK8tR5mZ2xF9jD4sL7gP3aE6u · адрес этого кошелька",
    );
}

/// The neighbour: a Platform line the core says nothing more of keeps the plain card — its
/// «Where» row stays and no «From» panel comes.
#[test]
fn a_line_without_detail_keeps_its_rows() {
    let (_turn, mut kit) = demo_app("activity", "en");
    kit.run_steps(2);
    crate::screens::open_card(&kit.ctx, 5);
    shown(&mut kit, "Where: Platform");
    assert!(!any_starts(&kit, "From:"));
}

/// The button whose name starts with `start`.
fn press(kit: &mut Kit, start: &str) {
    wait_for(kit, start, |kit| any_starts(kit, start));
    kit.query_all_by(|n| {
        n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with(start))
    })
    .last()
    .expect("the button")
    .click();
    kit.run_steps(3);
}

/// Mockup 03, AC-A2, AC-T6: a name says which identity registered it, where it went and what
/// proved it; its document's id is copied with feedback, and nothing is opened by itself.
#[test]
fn a_name_says_its_identity_and_copies_its_document_id() {
    let (_turn, mut kit) = demo_app("activity-card-name", "en");
    wait_for(&mut kit, "the identity", |kit| {
        any_starts(kit, "From: @satoshi · your identity")
    });
    shown(
        &mut kit,
        "To: Dash Platform · DPNS · domain document “nakamoto-dev.dash”",
    );
    shown(
        &mut kit,
        "Proved by Platform: the answer matches the GroveDB proof",
    );
    shown(&mut kit, "7Yd3kQw9nV2mRt6pLx4Hs8Fc1Bz5Jg3AeU6tNq2PvWk");
    assert!(!any_starts(&kit, "Where:"));
    press(&mut kit, "Copy the ID");
    wait_for(&mut kit, "Copied", |kit| has_text(kit, "Copied"));
    assert!(crate::links::opened().is_empty(), "no explorer by itself");
    press(&mut kit, "Open in the Platform explorer");
    let opened = crate::links::opened();
    assert_eq!(opened.len(), 1);
    assert!(
        opened[0].ends_with(
            "/document/7Yd3kQw9nV2mRt6pLx4Hs8Fc1Bz5Jg3AeU6tNq2PvWk?contract-id=GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec&document-type-name=domain"
        ),
        "{opened:?}"
    );
}

/// Mockup 04, AC-A3: not paid yet — it says so and leads nowhere.
#[test]
fn a_withdrawal_not_paid_waits_and_opens_nothing() {
    let (_turn, mut kit) = demo_app("activity-card-withdrawal-waiting", "en");
    wait_for(&mut kit, "waiting", |kit| {
        any_starts(kit, "Waiting for the Platform payout")
    });
    shown(
        &mut kit,
        &format!("Withdrawn: {}", format::amount(1_000_000)),
    );
    shown(&mut kit, &format!("Fee: {}", format::amount(215_053)));
    assert!(
        kit.query_all_by(|n| {
            n.role() == Role::Button
                && n.label()
                    .is_some_and(|l| l.starts_with("Waiting for the Platform"))
        })
        .next()
        .is_none(),
        "not a button"
    );
}

/// How the payout's card names its withdrawal; the row behind the card has no amount in its name.
fn back_to_the_withdrawal() -> String {
    format!(
        "Credit withdrawal to {} · {} · ",
        crate::text::short_address("yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f2c"),
        crate::text::signed_amount(-40_000_000)
    )
}

/// Mockup 05, AC-A3, AC-T11, AC-D4: the paid withdrawal's line opens its payout, whose line
/// leads back; each names the identity.
#[test]
fn a_paid_withdrawal_and_its_payout_open_each_other() {
    let (_turn, mut kit) = demo_app("activity-card-withdrawal-paid", "en");
    let paid = format!("Paid · block {}", group_thousands(1_283_802));
    press(&mut kit, &paid);
    let back = back_to_the_withdrawal();
    wait_for(&mut kit, "the payout's card", |kit| any_starts(kit, &back));
    assert!(any_starts(&kit, "From: @satoshi · your identity"));
    press(&mut kit, &back);
    wait_for(&mut kit, "the withdrawal again", |kit| {
        any_starts(kit, &paid)
    });
}

/// The same with the keyboard: Enter on the line opens it (AC-D4).
#[test]
fn enter_on_the_related_line_opens_it() {
    let (_turn, mut kit) = demo_app("activity-card-withdrawal-paid", "en");
    let paid = format!("Paid · block {}", group_thousands(1_283_802));
    wait_for(&mut kit, "the line", |kit| any_starts(kit, &paid));
    tab_to(&mut kit, &paid);
    enter(&mut kit);
    let back = back_to_the_withdrawal();
    wait_for(&mut kit, "the payout's card", |kit| any_starts(kit, &back));
}

/// AC-T5: «Open in Identity» works as the operation's identity, by the menu's command, and goes
/// to the Identity screen.
#[test]
fn open_in_identity_switches_to_that_identity() {
    let (_turn, mut kit) = demo_app("activity-card-name", "en");
    wait_for(&mut kit, "the identity", |kit| {
        any_starts(kit, "From: @satoshi · your identity")
    });
    let wallet = |kit: &Kit| {
        kit.state()
            .snapshot
            .clone()
            .and_then(|s| s.wallet)
            .expect("wallet")
    };
    let other = wallet(&kit)
        .identities
        .iter()
        .map(|c| c.id.clone())
        .find(|id| wallet(&kit).identity.is_some_and(|i| &i.id != id))
        .expect("a second identity");
    let app = kit.state_mut();
    if let Some(w) = app.snapshot.as_mut().and_then(|s| s.wallet.as_mut()) {
        for d in w.activity.iter_mut().filter_map(|a| a.detail.as_mut()) {
            if let Some(i) = d.identity.as_mut() {
                i.id = other.clone();
            }
        }
    }
    kit.run_steps(2);
    press(&mut kit, "Open in Identity");
    wait_for(&mut kit, "the Identity screen", |kit| {
        kit.state().screen == crate::screens::Screen::Identity
    });
    wait_for(&mut kit, "the other identity", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .and_then(|w| w.identity.as_ref())
            .is_some_and(|i| i.id == other)
    });
}

/// Mockup 06, BUG-57: a lock not taken says so in red's words and its line continues it.
#[test]
fn a_lock_not_added_leads_to_the_identity_screen() {
    let (_turn, mut kit) = demo_app("activity-card-lock-not-added", "en");
    shown(&mut kit, "From: Core · this wallet · asset lock");
    wait_for(&mut kit, "the identity", |kit| {
        any_starts(kit, "To: @satoshi · your identity")
    });
    shown(&mut kit, "Credits: not added");
    assert!(
        !any_starts(&kit, "Open in Identity"),
        "«Continue» is the way"
    );
    press(&mut kit, "The credits are not added yet");
    wait_for(&mut kit, "the Identity screen", |kit| {
        kit.state().screen == crate::screens::Screen::Identity
    });
}

/// Mockup 07: what went into the pool apart from the fee, from whom and to where.
#[test]
fn a_shield_shows_the_part_and_the_fee_apart() {
    let (_turn, mut kit) = demo_app("activity-card-shield", "en");
    shown(&mut kit, "To: This wallet's private pool");
    shown(
        &mut kit,
        &format!("Into the pool: {}", format::amount(248_788_300)),
    );
    shown(&mut kit, &format!("Fee: {}", format::amount(1_211_700)));
    assert!(
        !any_starts(&kit, "Copy the ID"),
        "a shield writes no document"
    );
}

/// FR-3: the pool's own lines say which way the money went; a payment out says it does not keep
/// its recipient.
#[test]
fn the_pools_lines_say_their_direction() {
    let (_turn, mut kit) = demo_app("activity-card-pool-sent", "en");
    shown(&mut kit, "From: This wallet's private pool");
    shown(
        &mut kit,
        "Part of the details of this operation was not saved",
    );
    let (_turn2, mut kit) = {
        drop(kit);
        drop(_turn);
        demo_app("activity-card-pool-received", "en")
    };
    shown(&mut kit, "To: This wallet's private pool");
    assert!(!any_starts(&kit, "Part of the details"));
}

/// FR-3: a top-up from this computer says how much it added.
#[test]
fn a_top_up_says_how_much_was_added() {
    let (_turn, mut kit) = demo_app("activity-card-top-up", "en");
    shown(
        &mut kit,
        &format!("Credits: {} were added", format::amount(49_990_000)),
    );
    assert!(!any_starts(&kit, "Part of the details"));
}

/// Mockup 08, AC-A6: an old entry draws no empty rows and says so once.
#[test]
fn an_old_entry_has_no_empty_rows_and_one_line() {
    let (_turn, mut kit) = demo_app("activity-card-old", "en");
    shown(
        &mut kit,
        "Part of the details of this operation was not saved",
    );
    shown(&mut kit, "To: @bob");
    assert!(!any_starts(&kit, "From:"));
    assert!(!any_starts(&kit, "Copy the ID"));
    assert!(!any_starts(&kit, "Open in Identity"));
}

/// Review r1 should 1: a screen reader hears «From» of a Core payment in the interface's
/// language, as it is shown — not the English key.
#[test]
fn the_from_of_a_payment_out_is_heard_in_the_language_shown() {
    let (_turn, mut kit) = demo_app("activity-card", "ru");
    wait_for(&mut kit, "«Откуда» in Russian", |kit| {
        any_starts(kit, "Откуда: Core · этот кошелёк")
    });
    assert!(!any_starts(&kit, "Откуда: Core · this wallet"));
}

/// FR-6, review r1 should 2 (D7): a card whose record lost part of the details says so in one
/// line; the same card whole does not.
#[test]
fn a_card_with_details_not_saved_says_so() {
    const LINE: &str = "Part of the details of this operation was not saved";
    let (_turn, mut kit) = demo_app("activity-card", "en");
    shown(&mut kit, "From: Core · this wallet · 3 inputs");
    assert!(!has_text(&kit, LINE));
    let app = kit.state_mut();
    app.dev.incomplete_details = true;
    if let Some(w) = app.snapshot.as_mut().and_then(|s| s.wallet.as_mut()) {
        for d in w.activity.iter_mut().filter_map(|a| a.detail.as_mut()) {
            d.incomplete = true;
        }
    }
    shown(&mut kit, LINE);
}

/// Review r1 should 3: in Arabic the «From → To» panel is mirrored as a whole — each row's
/// label stands to the right of its value, on the same line.
#[test]
fn in_arabic_the_label_of_a_side_stands_right_of_its_value() {
    let (_turn, mut kit) = demo_app("activity-card-self", "ar");
    let address = "yNBw5qT8mK2vL6pR9xC3sD1fG7hZ4aE2iReb";
    wait_for(&mut kit, "the «From» row", |kit| any_starts(kit, "من: "));
    kit.run_steps(2);
    let row = kit
        .query_all_by(|n| {
            n.label()
                .or(n.value())
                .is_some_and(|l| l.starts_with("من: "))
        })
        .next()
        .expect("the «From» row")
        .rect();
    fn texts(shape: &egui::Shape, found: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => {
                found.push((t.galley.text().to_owned(), t.visual_bounding_rect()))
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &kit.output().shapes {
        texts(&clipped.shape, &mut found);
    }
    let in_row: Vec<_> = found
        .into_iter()
        .filter(|(_, r)| row.expand(1.0).contains_rect(*r))
        .collect();
    let value = in_row
        .iter()
        .find(|(t, _)| t.contains(address))
        .expect("the address in the row")
        .1;
    let label = in_row
        .iter()
        .find(|(t, r)| !t.contains(address) && value.y_range().contains(r.center().y))
        .unwrap_or_else(|| panic!("the label on the address's line: {in_row:?}"))
        .1;
    assert!(
        label.left() >= value.right(),
        "label {label:?}, value {value:?}"
    );
}

// BUG-90: however many inputs and outputs an operation has, the card stays inside the window.

const WINDOW: egui::Rect =
    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1024.0, 768.0));

fn own_address(i: usize) -> String {
    format!("yNBw5qT8mK2vL6pR9xC3sD1fG7hZ4aE2i{i:03}")
}

/// The demo's transfer between own addresses (`activity-card-self`) as spent from `inputs`
/// addresses to `outputs`, on every snapshot.
fn tall_op(kit: &mut Kit, inputs: usize, outputs: usize) {
    if let Some(s) = kit.state_mut().snapshot.as_mut()
        && let Some(act) = s
            .wallet
            .as_mut()
            .and_then(|w| w.activity.iter_mut().find(|a| a.id == 17))
        && let Some(d) = act.detail.as_mut()
    {
        d.own_from = (0..inputs).map(own_address).collect();
        d.own_to = (0..outputs)
            .map(|i| (own_address(1000 + i), 1_000))
            .collect();
    }
}

/// Draws the tall operation for `frames`: the demo core's own snapshots come in between.
fn tall_frames(kit: &mut Kit, inputs: usize, outputs: usize, frames: usize) {
    for _ in 0..frames {
        tall_op(kit, inputs, outputs);
        kit.run_steps(1);
    }
}

/// Every node named exactly `name`.
fn named_all(kit: &Kit, name: &str) -> Vec<egui::Rect> {
    kit.query_all_by(|n| n.label().as_deref() == Some(name))
        .map(|n| n.rect())
        .collect()
}

/// The card's cross (top) and its «Close» button (bottom).
fn cross_and_close(kit: &Kit) -> (egui::Rect, egui::Rect) {
    let close = crate::i18n::spoken("Close");
    let mut both = named_all(kit, &close);
    both.sort_by(|a, b| a.top().total_cmp(&b.top()));
    assert_eq!(both.len(), 2, "the cross and «Close»: {both:?}");
    (both[0], both[1])
}

/// The whole card is in the window, its frame too: egui keeps the cross and «Close» on screen
/// even when the dialog's edge is cut off, so the dialog's padding is counted around them. The
/// cross stands in the middle of the 48 px row of the header.
fn assert_inside(kit: &Kit, window: egui::Rect) {
    const PADDING: f32 = 24.0;
    const HALF_THE_HEADER: f32 = 24.0;
    let (cross, close) = cross_and_close(kit);
    assert!(
        cross.center().y - HALF_THE_HEADER - PADDING >= window.top(),
        "the card's top edge: the cross {cross:?} of {window:?}"
    );
    assert!(
        close.bottom() + PADDING <= window.bottom(),
        "the card's bottom edge: «Close» {close:?} of {window:?}"
    );
    assert!(
        window.contains_rect(cross),
        "the cross: {cross:?} of {window:?}"
    );
    assert!(
        window.contains_rect(close),
        "«Close»: {close:?} of {window:?}"
    );
}

/// Presses the bottom «Close» and waits for the card to go.
fn close_card(kit: &mut Kit) {
    let close = crate::i18n::spoken("Close");
    let lowest = kit
        .query_all_by(|n| n.label().as_deref() == Some(close.as_str()))
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("«Close»");
    lowest.click();
    wait_for(kit, "the card closed", |kit| {
        kit.ctx
            .data(|d| d.get_temp::<u64>(egui::Id::new("activity-card")))
            .is_none()
    });
}

/// Turns the wheel over the card until `seen`, at most ten notches.
fn wheel_until(kit: &mut Kit, inputs: usize, outputs: usize, seen: impl Fn(&Kit) -> bool) {
    let (_, close) = cross_and_close(kit);
    kit.hover_at(egui::pos2(close.center().x, close.top() - 120.0));
    for _ in 0..10 {
        if seen(kit) {
            return;
        }
        kit.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -300.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        tall_frames(kit, inputs, outputs, 4);
    }
    assert!(seen(kit), "the wheel did not get there");
}

/// BUG-90: 25 inputs at 1024×768 are one line «Core · this wallet · 25 inputs» beside «Show all»;
/// the cross and «Close» are in the window and «Close» closes the card.
#[test]
fn twenty_five_inputs_are_one_line_and_the_buttons_stay_in_the_window() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    tall_frames(&mut kit, 25, 1, 8);
    assert!(
        has_text(&kit, "From: Core · this wallet · 25 inputs"),
        "the side is said in one line"
    );
    assert!(!any_starts(&kit, &own_address(0)), "no address listed yet");
    assert!(has_text(&kit, "Show all"));
    assert_inside(&kit, WINDOW);
    close_card(&mut kit);
}

/// BUG-90: «Show all» lists all 25 inside the card: the buttons stay put and the wheel reaches
/// the last address and the txid's buttons.
#[test]
fn show_all_lists_the_inputs_inside_the_card_and_the_wheel_reaches_them() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    tall_frames(&mut kit, 25, 1, 8);
    button(&kit, "Show all").click();
    tall_frames(&mut kit, 25, 1, 8);
    assert!(!has_text(&kit, "Show all"), "said once, then it is done");
    let first = format!("From: {} · this wallet's address", own_address(0));
    let last = format!("{} · this wallet's address", own_address(24));
    assert!(
        has_text(&kit, &first) && has_text(&kit, &last),
        "all 25 listed"
    );
    assert_inside(&kit, WINDOW);
    let reached = |kit: &Kit| {
        let (_, close) = cross_and_close(kit);
        let bottom = kit
            .query_all_by(|n| n.label().as_deref() == Some("Copy txid"))
            .next()
            .expect("«Copy txid»")
            .rect();
        bottom.bottom() <= close.top()
    };
    wheel_until(&mut kit, 25, 1, reached);
    assert_inside(&kit, WINDOW);
    // The bar has its gutter in the dialog's padding: the lines are as wide as without one.
    let moved = kit
        .query_all_by(|n| n.value().is_some_and(|v| v.starts_with("Moved: ")))
        .next()
        .expect("the row «Moved»")
        .rect();
    assert_eq!(moved.right(), cross_and_close(&kit).1.right());
    close_card(&mut kit);
}

/// BUG-90, outputs: a transfer to 25 own addresses is the same line under «To», and its «Show
/// all» too. The inputs' side, three addresses, stays listed.
#[test]
fn twenty_five_outputs_are_one_line_too() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    tall_frames(&mut kit, 3, 25, 8);
    assert!(has_text(&kit, "To: Core · this wallet · 25 addresses"));
    assert!(any_starts(
        &kit,
        "From: yNBw5qT8mK2vL6pR9xC3sD1fG7hZ4aE2i000"
    ));
    assert_inside(&kit, WINDOW);
    button(&kit, "Show all").click();
    tall_frames(&mut kit, 3, 25, 8);
    assert!(any_starts(&kit, &format!("To: {}", own_address(1000))));
    assert!(any_starts(&kit, &own_address(1024)));
    assert_inside(&kit, WINDOW);
}

/// BUG-90 in Arabic, both sides long: the same, mirrored; and in a window as small as a laptop's.
#[test]
fn in_arabic_and_in_a_small_window_the_long_card_still_fits() {
    let (_turn, mut kit) = demo_app("activity-card-self", "ar");
    tall_frames(&mut kit, 25, 25, 8);
    assert_inside(&kit, WINDOW);
    let all = crate::i18n::spoken("Show all");
    button(&kit, &all).click();
    tall_frames(&mut kit, 25, 25, 8);
    assert_inside(&kit, WINDOW);
    kit.set_size(egui::vec2(853.0, 533.0));
    tall_frames(&mut kit, 25, 25, 8);
    let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(853.0, 533.0));
    assert_inside(&kit, small);
    // The smallest window the app allows (R2).
    kit.set_size(egui::vec2(800.0, 440.0));
    tall_frames(&mut kit, 25, 25, 8);
    assert_inside(
        &kit,
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 440.0)),
    );
    close_card(&mut kit);
}

/// BUG-90: «Show all» belongs to the card it was pressed in; opened again it is one line.
#[test]
fn show_all_is_forgotten_when_the_card_closes() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    tall_frames(&mut kit, 25, 1, 8);
    button(&kit, "Show all").click();
    tall_frames(&mut kit, 25, 1, 8);
    assert!(any_starts(&kit, &format!("From: {}", own_address(0))));
    close_card(&mut kit);
    crate::screens::open_card(&kit.ctx.clone(), 17);
    tall_frames(&mut kit, 25, 1, 8);
    assert!(has_text(&kit, "From: Core · this wallet · 25 inputs"));
    assert!(has_text(&kit, "Show all"));
}

/// BUG-90: in a tall window the card stands in the middle; «Show all» makes it grow past the
/// window's bottom, and the dialog moves up so that its buttons stay on screen.
#[test]
fn a_card_grown_by_show_all_moves_up_to_stay_in_the_window() {
    let tall = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 1100.0));
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    kit.set_size(tall.size());
    tall_frames(&mut kit, 25, 1, 8);
    let (cross, _) = cross_and_close(&kit);
    assert!(cross.top() > 100.0, "not in the middle: {cross:?}");
    assert_inside(&kit, tall);
    button(&kit, "Show all").click();
    tall_frames(&mut kit, 25, 1, 8);
    assert_inside(&kit, tall);
}

/// Where the first node of this name is.
fn rect_named(kit: &Kit, name: &str) -> egui::Rect {
    named_all(kit, name)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("no «{name}»"))
}

/// One notch of `by` points over the card; `draw` shows what the wheel did.
fn turn_wheel(kit: &mut Kit, by: f32, draw: impl FnOnce(&mut Kit)) {
    let (_, close) = cross_and_close(kit);
    kit.hover_at(egui::pos2(close.center().x, close.top() - 60.0));
    kit.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, by),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    draw(kit);
}

/// BUG-90 r2: the ordinary card at 1024×768 is what it was before the area: nothing to scroll,
/// its lines as wide as «Close» goes, the txid on one line. Also in the languages where the card
/// only just fits (5 px from the window's edges).
#[test]
fn an_ordinary_card_at_1024_by_768_does_not_scroll() {
    for lang in ["en", "ru", "uk", "es", "fr", "pt-BR"] {
        let (_turn, mut kit) = demo_app("activity-card-self", lang);
        let copy_txid = crate::i18n::spoken("Copy txid");
        wait_for(&mut kit, "«Copy txid»", |kit| {
            kit.query_by_role_and_label(Role::Button, &copy_txid)
                .is_some()
        });
        kit.run_steps(8);
        let (_, close) = cross_and_close(&kit);
        let txid = kit
            .query_all_by(|n| n.value().is_some_and(|v| v.starts_with("9c1e04a7")))
            .next()
            .expect("the txid")
            .rect();
        assert!(txid.height() < 24.0, "{lang}: the txid wrapped: {txid:?}");
        let moved_row = format!("{}: ", crate::i18n::spoken("Moved"));
        let moved = kit
            .query_all_by(|n| n.value().is_some_and(|v| v.starts_with(&moved_row)))
            .next()
            .expect("the row «Moved»")
            .rect();
        assert_eq!(moved.right(), close.right(), "{lang}: a gutter");
        let copy = rect_named(&kit, &copy_txid);
        turn_wheel(&mut kit, -300.0, |kit| kit.run_steps(6));
        assert_eq!(
            rect_named(&kit, &copy_txid),
            copy,
            "{lang}: the wheel moved it"
        );
        assert_eq!(cross_and_close(&kit).1, close, "{lang}: «Close» moved");
    }
}

const SMALLEST: egui::Rect =
    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 440.0));

/// BUG-90 r2: «Add a note» at the fold of a small window: the field it opens is seen, not only
/// focused below the fold.
#[test]
fn the_note_field_opened_at_the_fold_is_seen() {
    let (_turn, mut kit) = demo_app("activity-card-add-note", "en");
    wait_for(&mut kit, "«Add a note»", |kit| {
        kit.query_by_role_and_label(Role::Button, "Add a note")
            .is_some()
    });
    kit.set_size(SMALLEST.size());
    kit.run_steps(6);
    let fold = |kit: &Kit| cross_and_close(kit).1.top() - 16.0;
    for _ in 0..40 {
        if rect_named(&kit, "Add a note").bottom() <= fold(&kit) {
            break;
        }
        turn_wheel(&mut kit, -8.0, |kit| kit.run_steps(6));
    }
    let add = rect_named(&kit, "Add a note");
    assert!(add.bottom() <= fold(&kit), "not at the fold: {add:?}");
    assert!(add.bottom() > fold(&kit) - 16.0, "not at the fold: {add:?}");
    button(&kit, "Add a note").click();
    kit.run_steps(6);
    let field = kit
        .query_all_by_label("Note · only you see it")
        .find(|n| {
            matches!(
                n.accesskit_node().role(),
                Role::TextInput | Role::MultilineTextInput
            )
        })
        .expect("the field");
    assert!(field.accesskit_node().is_focused(), "no focus in the field");
    let r = field.rect();
    // The field's frame, 10 px around the text, comes into view with it.
    assert!(r.bottom() + 10.0 <= fold(&kit), "below the fold: {r:?}");
}

/// BUG-90 r2: Tab through the card in a small window: what has the focus is in the visible part,
/// so Enter is never pressed blind (the explorer above all).
#[test]
fn tab_brings_what_it_lands_on_into_view() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    kit.set_size(SMALLEST.size());
    tall_frames(&mut kit, 25, 1, 8);
    let mut seen = Vec::new();
    for _ in 0..12 {
        let before = rect_named(&kit, "Copy txid");
        tab(&mut kit, false);
        tall_frames(&mut kit, 25, 1, 8);
        let name = focused(&kit).expect("the focus stays in the card");
        if name == "Close" {
            // The cross and «Close» are outside the scrolled part: it stays where it was.
            assert_eq!(rect_named(&kit, "Copy txid"), before);
            continue;
        }
        let (cross, close) = cross_and_close(&kit);
        let r = kit
            .query_all_by(|n| n.is_focused())
            .next()
            .expect("the focused node")
            .rect();
        assert!(
            r.top() >= cross.bottom() && r.bottom() <= close.top() - 16.0,
            "«{name}» {r:?} is out of view between {cross:?} and {close:?}"
        );
        seen.push(name);
    }
    for name in ["Add a note", "Copy txid", "Open in the explorer"] {
        assert!(seen.iter().any(|s| s == name), "no «{name}» in {seen:?}");
    }
}

/// BUG-90 r2: following the focus happens when it moves: the wheel can leave a focused button
/// behind, and it is not pulled back.
#[test]
fn the_wheel_can_leave_the_focused_button_behind() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    kit.set_size(SMALLEST.size());
    tall_frames(&mut kit, 25, 1, 8);
    for _ in 0..12 {
        if focused(&kit).as_deref() == Some("Copy txid") {
            break;
        }
        tab(&mut kit, false);
        tall_frames(&mut kit, 25, 1, 8);
    }
    assert_eq!(focused(&kit).as_deref(), Some("Copy txid"));
    let at = rect_named(&kit, "Copy txid");
    turn_wheel(&mut kit, 300.0, |kit| tall_frames(kit, 25, 1, 8));
    let up = rect_named(&kit, "Copy txid");
    assert!(up.top() > at.top() + 50.0, "the wheel did nothing: {up:?}");
    tall_frames(&mut kit, 25, 1, 8);
    assert_eq!(rect_named(&kit, "Copy txid"), up, "pulled back");
    assert_eq!(focused(&kit).as_deref(), Some("Copy txid"));
}

/// BUG-90 r2: a card scrolled down, closed and opened again starts at the top.
#[test]
fn a_reopened_card_starts_at_the_top() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    kit.set_size(SMALLEST.size());
    tall_frames(&mut kit, 25, 1, 8);
    let first = rect_named(&kit, "Copy txid");
    for _ in 0..6 {
        turn_wheel(&mut kit, -300.0, |kit| tall_frames(kit, 25, 1, 6));
    }
    let down = rect_named(&kit, "Copy txid");
    assert!(down.top() < first.top() - 50.0, "did not scroll: {down:?}");
    close_card(&mut kit);
    crate::screens::open_card(&kit.ctx.clone(), 17);
    tall_frames(&mut kit, 25, 1, 8);
    assert_eq!(rect_named(&kit, "Copy txid"), first);
}

/// BUG-90 r2: the summary counts addresses, not lines: a change address met twice is one.
#[test]
fn the_summary_counts_an_address_used_twice_once() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    for _ in 0..8 {
        tall_op(&mut kit, 3, 25);
        if let Some(s) = kit.state_mut().snapshot.as_mut()
            && let Some(d) = s
                .wallet
                .as_mut()
                .and_then(|w| w.activity.iter_mut().find(|a| a.id == 17))
                .and_then(|a| a.detail.as_mut())
        {
            // 25 outputs to 24 addresses.
            d.own_to[1].0 = d.own_to[0].0.clone();
        }
        kit.run_steps(1);
    }
    assert!(has_text(&kit, "To: Core · this wallet · 24 addresses"));
}

/// BUG-90 r2: the bar of a long card stands beside the lines, in the dialog's padding, and works
/// there: pressed near its bottom end, it takes the card down.
#[test]
fn the_bar_of_a_long_card_stands_beside_the_lines() {
    let (_turn, mut kit) = demo_app("activity-card-self", "en");
    kit.set_size(SMALLEST.size());
    tall_frames(&mut kit, 25, 1, 8);
    let at = rect_named(&kit, "Copy txid");
    let (_, close) = cross_and_close(&kit);
    let on_the_bar = egui::pos2(close.right() + 8.0, close.top() - 24.0);
    kit.hover_at(on_the_bar);
    tall_frames(&mut kit, 25, 1, 2);
    kit.drag_at(on_the_bar);
    tall_frames(&mut kit, 25, 1, 2);
    kit.drop_at(on_the_bar);
    tall_frames(&mut kit, 25, 1, 4);
    let down = rect_named(&kit, "Copy txid");
    assert!(
        down.top() < at.top() - 50.0,
        "the bar is not there: {down:?}"
    );
}
