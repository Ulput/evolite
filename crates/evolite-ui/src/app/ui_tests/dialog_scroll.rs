//! BUG-122: however low the window, a dialog stays in it. The header stays at the top, the
//! footer (the password, its line of error, the buttons) at the bottom, and the middle between
//! them scrolls, with a fade at its bottom while there is more below.

use super::*;
use crate::cli::PRESETS;
use crate::components::{DialogSeen, dialog_seen};

/// The windows of AC-T1: 800×600, the smallest the app allows (800×440) and 1280×720 at 150 %
/// (FEAT-0030's block of change).
const LOW: [[f32; 2]; 3] = [[800.0, 600.0], [800.0, 440.0], [853.0, 533.0]];
const SMALLEST: [f32; 2] = [800.0, 440.0];

fn window(kit: &Kit) -> egui::Rect {
    kit.ctx.content_rect()
}

/// The dialog drawn in the last frame.
fn seen(kit: &Kit) -> DialogSeen {
    dialog_seen(&kit.ctx).expect("a dialog drawn")
}

/// Where the dialog's own widgets of `role` are: not those of the page behind it.
fn in_dialog(kit: &Kit, role: Role) -> Vec<egui::Rect> {
    let dialog = seen(kit);
    kit.query_all_by_role(role)
        .filter(|n| {
            let mut node = n.accesskit_node().parent();
            while let Some(up) = node {
                if dialog.nodes.contains(&up.locate().0.0) {
                    return true;
                }
                node = up.parent();
            }
            false
        })
        .map(|n| n.rect())
        .collect()
}

/// Where the dialog's password fields are.
fn passwords(kit: &Kit) -> Vec<egui::Rect> {
    in_dialog(kit, Role::PasswordInput)
}

/// The buttons in the dialog's footer.
fn footer_buttons(kit: &Kit, footer: egui::Rect) -> Vec<egui::Rect> {
    in_dialog(kit, Role::Button)
        .into_iter()
        .filter(|r| footer.contains_rect(*r))
        .collect()
}

/// Everything AC-T1 asks of a dialog that is up: its footer, the buttons in it and every
/// password field are wholly in the window, and no password field is in the scrolled middle.
/// What is wrong, else nothing.
fn kept(kit: &Kit) -> Vec<String> {
    let window = window(kit);
    let dialog = seen(kit);
    let mut wrong = Vec::new();
    match dialog.footer {
        Some(footer) => {
            if !window.contains_rect(footer) {
                wrong.push(format!("the footer {footer:?} is out of {window:?}"));
            }
            if footer_buttons(kit, footer).is_empty() {
                wrong.push("no button in the footer".into());
            }
        }
        // It scrolls with buttons in the middle: a forgotten footer. A list to choose from
        // (the languages) has none to keep in sight.
        None => {
            if let Some(view) = dialog.scrolled {
                let buttons = in_dialog(kit, Role::Button)
                    .into_iter()
                    .filter(|r| r.top() >= view.top())
                    .count();
                if buttons > 0 {
                    wrong.push(format!(
                        "it scrolls with {buttons} buttons, and has no footer"
                    ));
                }
            }
        }
    }
    for field in passwords(kit) {
        if !window.contains_rect(field) {
            wrong.push(format!("a password field {field:?} is out of {window:?}"));
        }
        if dialog
            .scrolled
            .is_some_and(|view| field.top() < view.bottom())
        {
            wrong.push(format!(
                "a password field {field:?} scrolls with the middle"
            ));
        }
    }
    wrong
}

/// The password asked for every spend, so that the last look and the profile show their field.
fn with_password(w: &mut evolite_api::Wallet) {
    w.password_each_send = true;
}

/// Frames at `size` with the password asked, until the dialog has been drawn there twice.
fn at(kit: &mut Kit, size: [f32; 2]) {
    kit.set_size(egui::vec2(size[0], size[1]));
    show_wallet_edited(kit, 4, "the dialog at its size", with_password, |kit| {
        dialog_seen(&kit.ctx).is_some()
    });
}

/// AC-T1: «Check the payment» with coins and the password, the DashPay profile and an
/// operation's card: in each low window the main button and the password field are in it.
#[test]
fn the_tallest_dialogs_keep_their_password_and_buttons_in_a_low_window() {
    for preset in ["send-coins-confirm", "edit-profile", "activity-card"] {
        let (_turn, mut kit) = demo_app(preset, "en");
        settle(&mut kit);
        for size in LOW {
            at(&mut kit, size);
            let wrong = kept(&kit);
            assert!(wrong.is_empty(), "{preset} at {size:?}: {wrong:?}");
            if preset != "activity-card" {
                assert_eq!(passwords(&kit).len(), 1, "{preset}: its password field");
            }
        }
        // The smallest window is too low for any of them whole: the middle scrolls.
        at(&mut kit, SMALLEST);
        assert!(seen(&kit).scrolled.is_some(), "{preset}: nothing scrolls");
    }
}

/// AC-T1, every dialog: each preset that opens one, in the smallest window, keeps its footer,
/// its buttons and its password in it. A new dialog with a password and no footer fails here.
#[test]
fn every_dialog_keeps_its_footer_in_the_smallest_window() {
    let mut wrong = Vec::new();
    let mut dialogs = 0;
    for preset in PRESETS {
        // The staged sync never goes quiet; it has no dialog.
        if matches!(*preset, "sync" | "sync-nodes") {
            continue;
        }
        let (_turn, mut kit) = demo_app(preset, "en");
        if preset.starts_with("connect") {
            wait_for(&mut kit, "the app's request", |kit| {
                dialog_seen(&kit.ctx).is_some()
            });
        }
        settle(&mut kit);
        if dialog_seen(&kit.ctx).is_none() {
            continue;
        }
        dialogs += 1;
        kit.set_size(egui::vec2(SMALLEST[0], SMALLEST[1]));
        kit.run_steps(4);
        if dialog_seen(&kit.ctx).is_none() {
            wrong.push(format!("{preset}: the dialog went at 800×440"));
            continue;
        }
        wrong.extend(
            kept(&kit)
                .into_iter()
                .map(|w| format!("{preset} ({:?}): {w}", seen(&kit).id)),
        );
    }
    assert!(dialogs > 30, "only {dialogs} presets open a dialog");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// AC-T2: a short dialog in a tall window is as it was: nothing scrolls, no fade, and its
/// buttons end the dialog with nothing added under them.
#[test]
fn a_short_dialog_in_a_tall_window_is_as_before() {
    let (_turn, mut kit) = demo_app("wallet-rename", "en");
    settle(&mut kit);
    kit.set_size(egui::vec2(1280.0, 800.0));
    kit.run_steps(4);
    let dialog = seen(&kit);
    assert!(dialog.scrolled.is_none(), "it scrolls");
    assert!(dialog.fade.is_none(), "a fade");
    let lowest = kit
        .query_all_by_role(Role::Button)
        .map(|n| n.rect())
        .filter(|r| dialog.rect.contains_rect(*r))
        .map(|r| r.bottom())
        .fold(f32::MIN, f32::max);
    assert!(
        (dialog.rect.bottom() - lowest).abs() < 1.0,
        "space under the buttons: {} below {lowest}",
        dialog.rect.bottom()
    );
}

/// AC-T5: the window goes from 1024×768 down to 800×440 with «Check the payment» open: a frame
/// later its buttons and password are in the window, and its top is not above it.
#[test]
fn a_dialog_open_while_the_window_shrinks_stays_in_it() {
    let (_turn, mut kit) = demo_app("send-coins-confirm", "en");
    settle(&mut kit);
    at(&mut kit, [1024.0, 768.0]);
    kit.set_size(egui::vec2(SMALLEST[0], SMALLEST[1]));
    kit.step();
    kit.step();
    let wrong = kept(&kit);
    assert!(wrong.is_empty(), "{wrong:?}");
    assert!(
        seen(&kit).rect.top() >= window(&kit).top(),
        "above the window"
    );
}

/// AC-T6: Tab through the middle of the DashPay profile in the smallest window (its picture
/// link, name and «Bio»): what it lands on is in the visible part; the password field, in the
/// footer, never moves the middle. A button below the fold brought into view (fields scroll to
/// their cursor themselves) is `card::tab_brings_what_it_lands_on_into_view`, on this same rule.
#[test]
fn tab_brings_into_view_and_the_password_moves_nothing() {
    let (_turn, mut kit) = demo_app("edit-profile", "en");
    settle(&mut kit);
    at(&mut kit, SMALLEST);
    let field = passwords(&kit)[0];
    let mut seen_in_middle = 0;
    for _ in 0..16 {
        let before = seen(&kit).offset;
        tab(&mut kit, false);
        kit.run_steps(4);
        let Some(focus) = kit
            .query_all_by(|n| n.is_focused())
            .next()
            .map(|n| n.rect())
        else {
            continue;
        };
        let dialog = seen(&kit);
        let view = dialog.scrolled.expect("the middle scrolls");
        if focus == passwords(&kit)[0] {
            assert_eq!(dialog.offset, before, "the password moved the middle");
            assert_eq!(focus, field, "the password field moved");
        } else if focus.top() < view.bottom() && focus.bottom() > view.top() {
            assert!(
                focus.top() >= view.top() - 1.0 && focus.bottom() <= view.bottom() + 1.0,
                "{focus:?} is cut by the middle {view:?}"
            );
            seen_in_middle += 1;
        }
    }
    assert!(seen_in_middle > 0, "Tab never went through the middle");
}

/// Turns the wheel over the dialog's middle.
fn wheel(kit: &mut Kit, by: f32) {
    let view = seen(kit).scrolled.expect("the middle scrolls");
    kit.hover_at(view.center());
    kit.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, by),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    kit.run_steps(4);
}

/// AC-T7, AC-D3: the fade shows while there is more below, at the middle's bottom, and goes at
/// its end; the wheel reaches the end.
#[test]
fn the_fade_shows_only_while_there_is_more_below() {
    let (_turn, mut kit) = demo_app("send-coins-confirm", "en");
    settle(&mut kit);
    at(&mut kit, SMALLEST);
    let view = seen(&kit).scrolled.expect("the middle scrolls");
    let fade = seen(&kit).fade.expect("a fade at the top");
    assert_eq!(fade.bottom(), view.bottom(), "the fade ends the middle");
    assert_eq!(fade.height(), 16.0);
    for _ in 0..20 {
        if seen(&kit).fade.is_none() {
            break;
        }
        wheel(&mut kit, -200.0);
    }
    assert!(seen(&kit).fade.is_none(), "a fade at the end");
    assert!(seen(&kit).offset > 0.0, "the wheel did nothing");
}

/// Presses the dialog's bar (the side padding beside its middle, `x` on the screen) near the
/// middle's bottom and lets go: whether the middle went down.
fn press_the_bar(kit: &mut Kit, x: f32) -> bool {
    let view = seen(kit).scrolled.expect("the middle scrolls");
    let at = egui::pos2(x, view.bottom() - 12.0);
    let before = seen(kit).offset;
    kit.hover_at(at);
    kit.run_steps(2);
    kit.drag_at(at);
    kit.run_steps(2);
    kit.drop_at(at);
    kit.run_steps(4);
    seen(kit).offset > before + 20.0
}

/// AC-T8, AC-D5: in Arabic the bar stands on the left of the middle: pressed there, it takes the
/// middle down; in English, on the right. The fade is the same.
#[test]
fn the_bar_is_on_the_left_in_arabic() {
    for lang in ["en", "ar"] {
        let (_turn, mut kit) = demo_app("send-coins-confirm", lang);
        settle(&mut kit);
        at(&mut kit, SMALLEST);
        assert!(seen(&kit).fade.is_some(), "{lang}: no fade");
        // The layout is left to right; the picture and the pointer are turned over in Arabic.
        let right = seen(&kit).scrolled.expect("the middle scrolls").right() + 5.0;
        let x = if lang == "ar" {
            window(&kit).width() - right
        } else {
            right
        };
        assert!(press_the_bar(&mut kit, x), "{lang}: no bar at {x}");
    }
}

/// The dialog's frame from the window's top and bottom edges: its content less the 24 of
/// padding and the 1 of stroke.
fn margins(kit: &Kit) -> (f32, f32) {
    let frame = seen(kit).rect.expand(25.0);
    let window = window(kit);
    (frame.top() - window.top(), window.bottom() - frame.bottom())
}

/// The designer (BUG-122 review): a dialog that scrolls stands 16 from the window's top and
/// bottom, so the screen behind shows around it.
#[test]
fn a_dialog_that_scrolls_stands_16_from_the_window() {
    let (_turn, mut kit) = demo_app("send-coins-confirm", "ru");
    settle(&mut kit);
    at(&mut kit, SMALLEST);
    kit.run_steps(4);
    assert!(seen(&kit).scrolled.is_some(), "nothing scrolls");
    let (top, bottom) = margins(&kit);
    let near = |m: f32| (m - crate::components::MARGIN_SCROLL).abs() <= 1.5;
    assert!(
        near(top) && near(bottom),
        "{top} above, {bottom} below: {:?}",
        seen(&kit)
    );
}

/// The designer (BUG-122 review): a dialog that fits whole may come as close as 4 to the
/// window and does not scroll: the ordinary card at 1024×768 in ru is as before (BUG-90).
#[test]
fn a_dialog_that_fits_may_come_4_from_the_window() {
    let (_turn, mut kit) = demo_app("activity-card-self", "ru");
    let copy_txid = crate::i18n::spoken("Copy txid");
    wait_for(&mut kit, "«Copy txid»", |kit| {
        kit.query_by_role_and_label(Role::Button, &copy_txid)
            .is_some()
    });
    kit.run_steps(8);
    assert!(seen(&kit).scrolled.is_none(), "it scrolls");
    let (top, bottom) = margins(&kit);
    let fit = crate::components::MARGIN_FIT;
    assert!(
        top >= fit - 0.5 && bottom >= fit - 0.5,
        "{top} above, {bottom} below"
    );
    assert!(
        top.min(bottom) < crate::components::MARGIN_SCROLL,
        "it would have fitted within 16 anyway: {top}, {bottom}"
    );
}

/// The designer (BUG-122): a new opening shows the middle from its top, even when the focus
/// starts in a field further down (rule b′). App Connect's request shows whose request it is
/// first, its limit field focused below.
#[test]
fn a_new_opening_shows_the_middle_from_its_top() {
    let (_turn, mut kit) = demo_app("connect", "en");
    kit.set_size(egui::vec2(SMALLEST[0], SMALLEST[1]));
    wait_for(&mut kit, "the app's request", |kit| {
        dialog_seen(&kit.ctx).is_some()
    });
    kit.run_steps(20);
    let dialog = seen(&kit);
    assert!(dialog.scrolled.is_some(), "nothing scrolls");
    assert!(focused(&kit).is_some(), "no focus on opening");
    assert_eq!(dialog.offset, 0.0, "the middle moved to the focus");
}

/// FEAT-0030 screens review r1, P3 1 (a guard for BUG-126): in Arabic a field in the scrolling
/// middle of a dialog keeps its Latin text left to right in its place, the cursor after the
/// last letter. The RTL mirror keeps a field only when the clip egui painted it with is the one
/// `rtl::text_edit` reckons, to the last bit: an egui that rounds it otherwise would mirror the
/// field — the cursor would jump to the text's first letter.
#[test]
fn a_field_in_a_scrolling_middle_keeps_its_text_left_to_right_in_arabic() {
    const TYPED: &str = "yXk3Tq9mD8";
    let (_turn, mut kit) = demo_app("edit-profile", "ar");
    settle(&mut kit);
    at(&mut kit, SMALLEST);
    let view = seen(&kit).scrolled.expect("the middle scrolls");
    let field = in_dialog(&kit, Role::TextInput)
        .into_iter()
        .find(|r| view.contains_rect(*r))
        .expect("a field in the middle");
    kit.query_all_by_role(Role::TextInput)
        .find(|n| n.rect() == field)
        .expect("the field's node")
        .click();
    kit.run_steps(2);
    kit.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    kit.run_steps(1);
    kit.query_all_by_role(Role::TextInput)
        .find(|n| n.rect() == field)
        .expect("the field's node")
        .type_text(TYPED);
    kit.run_steps(3);
    assert!(seen(&kit).scrolled.is_some(), "still scrolling");
    let mut text = None;
    let mut cursors = Vec::new();
    fn walk(
        s: &egui::Shape,
        text: &mut Option<egui::Rect>,
        cursors: &mut Vec<f32>,
        field: egui::Rect,
    ) {
        match s {
            egui::Shape::Text(t) if t.galley.job.text == TYPED => {
                *text = Some(t.visual_bounding_rect())
            }
            egui::Shape::LineSegment { points: [a, b], .. }
                if a.x == b.x && b.y - a.y > 8.0 && field.contains(*a) =>
            {
                cursors.push(a.x)
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, cursors, field)),
            _ => {}
        }
    }
    for c in &kit.output().shapes {
        walk(&c.shape, &mut text, &mut cursors, field.expand(2.0));
    }
    let text = text.expect("the typed text painted");
    assert!(
        field.expand(1.0).contains_rect(text),
        "the text {text:?} in its field {field:?}"
    );
    let cursor = cursors.first().copied().expect("the cursor");
    assert!(
        (cursor - text.max.x).abs() < 4.0,
        "the cursor at {cursor} after the last letter ({text:?}), not reflected to the first"
    );
}
