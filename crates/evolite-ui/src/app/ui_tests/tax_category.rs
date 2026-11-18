//! FEAT-0012 (AC-T7): an operation's tax category in its card — the pair its direction allows,
//! a click or an arrow switching it at once, a refusal putting it back with the reason, one Tab
//! stop for the pair and one for «?», the window «?» opens, and Arabic mirrored but for the
//! codes of the CSV.

use super::*;

/// The pair, as a screen reader hears it: «Tax category» with the half shown as its value.
fn pair(kit: &Kit) -> Option<egui_kittest::Node<'_>> {
    let name = crate::i18n::spoken("Tax category");
    kit.query_all_by_role(Role::RadioGroup)
        .find(|n| n.accesskit_node().label().as_deref() == Some(name.as_ref()))
}

/// The half the pair shows, in English.
fn shown(kit: &Kit) -> Option<String> {
    pair(kit)?.accesskit_node().value()
}

/// The card's row whole in the window: the card is taller than 768 with it (the designer's
/// note) and scrolls.
fn tall(kit: &mut Kit) {
    kit.set_size(egui::vec2(1024.0, 1300.0));
    kit.run_steps(4);
}

/// A click on the right or the left half of the pair.
fn click_half(kit: &mut Kit, right: bool) {
    let rect = pair(kit).expect("the pair").rect();
    let x = if right {
        rect.right() - rect.width() / 4.0
    } else {
        rect.left() + rect.width() / 4.0
    };
    let at = egui::pos2(x, rect.center().y);
    let input = kit.input_mut();
    input.events.push(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        input.events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    kit.run_steps(3);
}

/// Whether the demo's line `id` is marked a transfer in the snapshot.
fn marked(kit: &Kit, id: u64) -> bool {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| w.activity.iter().find(|a| a.id == id))
        .is_some_and(|a| a.tax_transfer)
}

/// R-2, R-3 (AC-A3, AC-D2): a payment out offers «Expense | Transfer out», one in «Income |
/// Transfer in», each on the one by the sign, with the line about the CSV under it.
#[test]
fn the_pair_is_the_one_the_direction_allows() {
    for (preset, by_sign, other) in [
        ("activity-card", "Expense", "Transfer in"),
        ("activity-card-received", "Income", "Expense"),
    ] {
        let (_turn, mut kit) = demo_app(preset, "en");
        tall(&mut kit);
        wait_for(&mut kit, "the tax category", |kit| pair(kit).is_some());
        assert_eq!(shown(&kit).as_deref(), Some(by_sign), "{preset}");
        assert!(has_words(&kit, "Only for the CSV export"), "{preset}");
        assert!(!has_words(&kit, other), "{preset}: no other pair");
    }
}

/// UC-1, behaviour 1 (AC-D3): a click on the other half switches at once — no «Save» — and the
/// line under the row says what a transfer means and its code; a click back undoes it.
#[test]
fn a_click_switches_at_once_and_the_line_tells_the_code() {
    let (_turn, mut kit) = demo_app("activity-card", "en");
    tall(&mut kit);
    wait_for(&mut kit, "the tax category", |kit| pair(kit).is_some());
    click_half(&mut kit, true);
    wait_for(&mut kit, "the transfer in the snapshot", |kit| {
        marked(kit, 3)
    });
    kit.run_steps(2);
    assert_eq!(shown(&kit).as_deref(), Some("Transfer out"));
    assert!(has_words(&kit, "In the CSV: Transfer-out"));
    assert!(!has_words(&kit, "Only for the CSV export"));
    click_half(&mut kit, false);
    wait_for(&mut kit, "the mark taken off", |kit| !marked(kit, 3));
    kit.run_steps(2);
    assert_eq!(shown(&kit).as_deref(), Some("Expense"));
}

/// UC-2: a payment in switched to «Transfer in» says its code.
#[test]
fn a_payment_in_becomes_a_transfer_in() {
    let (_turn, mut kit) = demo_app("activity-card-tax-received", "en");
    tall(&mut kit);
    wait_for(&mut kit, "the transfer in", |kit| {
        shown(kit).as_deref() == Some("Transfer in")
    });
    assert!(has_words(&kit, "In the CSV: Transfer-in"));
}

/// Behaviour 2: a change the core refused puts the pair back to the snapshot's and says why,
/// as the notes do.
#[test]
fn a_refused_change_puts_the_pair_back_and_says_why() {
    let (_turn, mut kit) = demo_app("activity-card-tax-error", "en");
    tall(&mut kit);
    wait_for(&mut kit, "the tax category", |kit| pair(kit).is_some());
    assert_eq!(shown(&kit).as_deref(), Some("Expense"));
    assert!(has_words(
        &kit,
        "The category was not saved: there is no space left on the disk."
    ));
    button(&kit, "Details");
    // The next change tries again, and the reason goes.
    click_half(&mut kit, true);
    wait_for(&mut kit, "the transfer kept", |kit| marked(kit, 3));
    kit.run_steps(2);
    assert!(!has_words(&kit, "The category was not saved"));
}

/// Behaviour 3: the pair is one Tab stop and «?» another; the arrows switch the pair and keep
/// the focus on it, Space switches too.
#[test]
fn the_pair_is_one_tab_stop_and_the_arrows_switch_it() {
    let (_turn, mut kit) = demo_app("activity-card", "en");
    tall(&mut kit);
    wait_for(&mut kit, "the tax category", |kit| pair(kit).is_some());
    tab_to(&mut kit, "What the tax category is");
    tab(&mut kit, false);
    assert_eq!(focused(&kit).as_deref(), Some("Tax category"));
    tab(&mut kit, false);
    assert_ne!(
        focused(&kit).as_deref(),
        Some("Tax category"),
        "one stop for both halves"
    );
    tab(&mut kit, true);
    assert_eq!(focused(&kit).as_deref(), Some("Tax category"));
    kit.key_press(Key::ArrowRight);
    wait_for(&mut kit, "→ Transfer out", |kit| marked(kit, 3));
    kit.run_steps(2);
    assert_eq!(shown(&kit).as_deref(), Some("Transfer out"));
    assert_eq!(
        focused(&kit).as_deref(),
        Some("Tax category"),
        "the focus stays"
    );
    kit.key_press(Key::ArrowLeft);
    wait_for(&mut kit, "← Expense", |kit| !marked(kit, 3));
    kit.key_press(Key::Space);
    wait_for(&mut kit, "Space switches", |kit| marked(kit, 3));
}

/// UC-4 (AC-D4): «?» opens the window over the card — the four categories with their codes —
/// and «Got it» closes it, the card staying; Escape closes the window only.
#[test]
fn the_question_mark_opens_the_explainer_and_got_it_closes_it() {
    let (_turn, mut kit) = demo_app("activity-card", "en");
    tall(&mut kit);
    wait_for(&mut kit, "«?»", |kit| {
        kit.query_by_role_and_label(Role::Button, "What the tax category is")
            .is_some()
    });
    button(&kit, "What the tax category is").click();
    wait_for(&mut kit, "«Got it»", |kit| {
        kit.query_by_role_and_label(Role::Button, "Got it")
            .is_some()
    });
    for code in ["Income", "Expense", "Transfer-in", "Transfer-out"] {
        assert!(has_text(&kit, code), "{code}");
    }
    assert!(has_words(&kit, "evolite is not a tax adviser"));
    button(&kit, "Got it").click();
    // The window fades out over a few frames.
    wait_for(&mut kit, "the window closed", |kit| {
        kit.query_by_role_and_label(Role::Button, "Got it")
            .is_none()
    });
    assert!(pair(&kit).is_some(), "the card stays");
    button(&kit, "What the tax category is").click();
    wait_for(&mut kit, "«Got it» again", |kit| {
        kit.query_by_role_and_label(Role::Button, "Got it")
            .is_some()
    });
    kit.key_press(Key::Escape);
    wait_for(&mut kit, "the window closed by Escape", |kit| {
        kit.query_by_role_and_label(Role::Button, "Got it")
            .is_none()
    });
    assert!(pair(&kit).is_some(), "Escape closed the window only");
}

/// AC-D5: in Arabic the row is mirrored — the label right of the pair — but a code of the CSV
/// reads as it is written.
#[test]
fn in_arabic_the_row_is_mirrored_but_not_the_codes() {
    let (_turn, mut kit) = demo_app("activity-card-tax-transfer", "ar");
    tall(&mut kit);
    let out = crate::i18n::spoken("Transfer out");
    wait_for(&mut kit, "the transfer", |kit| {
        shown(kit).as_deref() == Some(out.as_str())
    });
    kit.run_steps(2);
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
    let label = crate::i18n::t("Tax category").into_owned();
    let find = |text: &str| {
        found
            .iter()
            .find(|(t, _)| t == text)
            .unwrap_or_else(|| panic!("«{text}» drawn: {found:?}"))
            .1
    };
    let label = find(&label);
    // The pair's first half, «Expense», is drawn left of its label in English.
    let half = find(&crate::i18n::t("Expense"));
    assert!(
        label.left() >= half.right(),
        "label {label:?}, half {half:?}"
    );
    assert!(
        found.iter().any(|(t, _)| t.contains("Transfer-out")),
        "the code as written"
    );
}
