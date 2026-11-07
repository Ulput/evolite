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

/// The neighbour: a Platform operation's card is as it was (5b changes it) — its «Where» row
/// stays and no «From» panel comes.
#[test]
fn a_platform_card_keeps_its_rows() {
    let (_turn, mut kit) = demo_app("activity", "en");
    kit.run_steps(2);
    crate::screens::open_card(&kit.ctx, 7);
    shown(&mut kit, "Where: Platform");
    assert!(!any_starts(&kit, "From:"));
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
