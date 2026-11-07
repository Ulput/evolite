//! The Send form (branch S: BUG-32, 35, 36, BUG-39 №5–7, 9): what a person sees on it.

use super::*;
use crate::i18n::{Lang, t_in};
use evolite_api::CoreError;

/// What the core says of a typo in an address (`real/convert.rs`, `not_an_address`).
const NOT_AN_ADDRESS: &str =
    "This is not a Dash address: a typo or an extra character — the checksum does not match";

/// Every label whose words are `text`, as drawn.
fn labels<'k>(kit: &'k Kit, text: &'k str) -> Vec<egui_kittest::Node<'k>> {
    kit.query_all(By::new().role(Role::Label).value(text))
        .collect()
}

/// "To" holds `to` and the core has answered for it with `preview`.
fn typed(kit: &mut Kit, to: &str, preview: Result<String, CoreError>) {
    let send = &mut kit.state_mut().send;
    send.recipient = to.into();
    send.previewed = to.into();
    send.preview = Some(preview);
    kit.run_steps(3);
}

/// BUG-35: the core's word for a typo in the address is said in Russian on a Russian screen,
/// with the full stop the form puts after it.
#[test]
fn a_typo_in_the_address_is_said_in_the_language_on_screen() {
    let (_turn, mut kit) = demo_app("send", "ru");
    typed(
        &mut kit,
        "yNotAnAddress123",
        Err(CoreError::NotAvailable(NOT_AN_ADDRESS.into())),
    );
    let russian = format!("{}.", t_in(Lang::Ru, NOT_AN_ADDRESS));
    assert!(russian.starts_with("Это не адрес Dash"), "{russian}");
    assert_eq!(labels(&kit, &russian).len(), 1, "«{russian}» on screen");
    assert!(!has_text(&kit, &format!("{NOT_AN_ADDRESS}.")));
}

/// BUG-36, its steps: while the wallet syncs, «To» = the wallet's own address and «Max». The
/// long «wait for the sync» line stays in the form's column — it ends before «Summary»
/// begins — and the whole «Summary» card stays inside the 1024 px window. Its rows are
/// painted, not in the tree: the card is the heading's left edge plus its inner width.
/// Before the fix the line widened the form and the card's right edge stood at ~1243.
#[test]
fn the_wait_for_sync_line_keeps_the_summary_in_the_window() {
    /// The summary card's inner width (`summary_w - 50` in `screens/send.rs`).
    const SUMMARY_INNER: f32 = 270.0;
    let (_turn, mut kit) = demo_app("send-syncing", "ru");
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0));
    let own = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.core_address.clone())
        .expect("the demo wallet");
    typed(&mut kit, &own, Ok(own.clone()));
    kit.get_by_role_and_label(Role::Button, &crate::i18n::spoken("Max"))
        .click();
    kit.run_steps(3);
    let hint = t_in(Lang::Ru, crate::text::WAIT_FOR_SYNC).into_owned();
    let hints = labels(&kit, &hint);
    assert_eq!(hints.len(), 1, "«{hint}» on screen");
    let line = hints[0].rect();
    let summary = words(&kit, "Итог").rect();
    eprintln!("window {window:?}, the line {line:?}, «Итог» {summary:?}");
    assert!(
        line.right() <= summary.left(),
        "the line ends at {}, «Итог» begins at {}",
        line.right(),
        summary.left()
    );
    let card_right = summary.left() + SUMMARY_INNER;
    assert!(
        window.contains_rect(summary) && card_right <= window.right(),
        "«Итог» {summary:?}: its card ends at {card_right}, past the window {window:?}"
    );
}

/// BUG-39 №9: the wallet's own address is not offered for the address book; it is said to be
/// the wallet's own. Someone else's address still is offered.
#[test]
fn the_wallets_own_address_is_not_offered_for_the_address_book() {
    let (_turn, mut kit) = demo_app("send", "en");
    let own = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.core_address.clone())
        .expect("the demo wallet");
    typed(&mut kit, &own, Ok(own.clone()));
    assert!(!has_text(&kit, "Save to the address book"));
    assert!(has_text(
        &kit,
        "This is an address of this wallet: the money stays yours, only the fee goes."
    ));

    let theirs = "yNBw8AS5PhKdSm6ytP3uAVEsx5PjCiReb4";
    typed(&mut kit, theirs, Ok(theirs.into()));
    assert!(has_text(&kit, "Save to the address book"));
}

/// BUG-68: the wallet's own address pasted with spaces around is its own too — the form asks
/// the core about the trimmed address and says so, not «Save to the address book».
#[test]
fn the_wallets_own_address_with_spaces_around_is_its_own() {
    let (_turn, mut kit) = demo_app("send", "en");
    let own = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.core_address.clone())
        .expect("the demo wallet");
    kit.state_mut().send.recipient = format!("  {own}  ");
    let said = "This is an address of this wallet: the money stays yours, only the fee goes.";
    wait_for(&mut kit, "the own-address line", |kit| has_text(kit, said));
    assert!(!has_text(&kit, "Save to the address book"));
}

/// BUG-67: the sources of «From» stand in one row of equal columns, a Core balance with all
/// eight decimals too and the other two long as well (1024 × 768, the rail), in Russian as the
/// bug was found and in English. Before the fix «Приватный пул» went to a second row.
#[test]
fn the_sources_stay_in_one_row_of_equal_columns() {
    for (lang, names) in [
        ("ru", ["Core", "Кредиты", "Приватный пул"]),
        ("en", ["Core", "Credits", "Private pool"]),
    ] {
        let (_turn, mut kit) = demo_app("send", lang);
        for _ in 0..5 {
            if let Some(w) = kit
                .state_mut()
                .snapshot
                .as_mut()
                .and_then(|s| s.wallet.as_mut())
            {
                w.balances.core = 391_724_464;
                // The other two long as well: the widest the row gets.
                w.balances.credits = 123_456_789;
                w.balances.shielded = 98_765_432;
            }
            kit.run_steps(1);
        }
        let cards: Vec<_> = kit
            .query_all_by_role(Role::RadioButton)
            .filter(|n| {
                n.accesskit_node()
                    .label()
                    .is_some_and(|l| names.contains(&l.as_str()))
            })
            .map(|n| n.rect())
            .collect();
        assert_eq!(cards.len(), 3, "{lang}: {names:?}: {cards:?}");
        let top = cards[0].top();
        assert!(
            cards.iter().all(|r| (r.top() - top).abs() < 0.5),
            "{lang}: one row: {cards:?}"
        );
        let width = cards[0].width();
        assert!(
            cards.iter().all(|r| (r.width() - width).abs() < 0.5),
            "{lang}: equal columns: {cards:?}"
        );
    }
}
