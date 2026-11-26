//! BUG-274 (QA: «Add a name» stayed open after the name was registered, the same name «available»
//! and «Register» on; «Unshield» the same), the whole class (TEAM.md §8d, 2): every dialog that
//! pays, once its payment has gone through, closes or shows what was done — never its pay button
//! on again for the same thing. A second press sends nothing.

use super::*;
use crate::cli::PRESETS;
use crate::components::dialog_seen;

/// The dialog drawn in the frame just run, if any: its record from this frame, not the last.
fn drawn_now(kit: &mut Kit) -> Option<crate::components::DialogSeen> {
    let before = dialog_seen(&kit.ctx).map(|d| d.pass);
    kit.run_steps(1);
    dialog_seen(&kit.ctx).filter(|d| Some(d.pass) != before)
}

fn type_password(kit: &mut Kit, password: &str) {
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field")
        .type_text(password);
    kit.run_steps(2);
}

/// TECH-paid-once-gaps: the presets whose act is off by design — each said why. Their dialogs
/// pay in other presets; any other paying dialog whose act stays off fails the test (a dialog
/// the test does not reach is a gap, not a pass).
const DESIGNED_OFF: &[(&str, &str)] = &[
    ("send-confirm-wait", "the chosen coins are still being read"),
    (
        "send-mixed-refused-confirm",
        "the excess is over the bound: refused",
    ),
    ("top-up-bad-amount", "an amount that is not one"),
    ("top-up-many-coins", "coins that cannot make the lock"),
    ("top-up-not-final", "coins not final yet"),
    ("unshield-bad-amount", "an amount that is not one"),
    ("shield", "a shield already running"),
    (
        "add-name-fund-rose",
        "the contest fund rose since it was shown: refused, the new one said",
    ),
];

/// The centre of an AccessKit box, in the window's points.
fn centre(r: egui::accesskit::Rect) -> egui::Pos2 {
    egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32)
}

/// What a person types into a field of a paying dialog, by its name: a recipient's Dash name,
/// a token's name in words, an amount anywhere else; `None` for a field that is not the
/// payment's (a lookup on the page behind, «Check an identity»).
fn filler(label: &str) -> Option<&'static str> {
    match label {
        "Check an identity" => None,
        "One" | "Many" => Some("Coffee"),
        l if l.contains("ecipient") || l == "@name" => Some("@alice"),
        _ => Some("0.01"),
    }
}

/// What a person fills before the act turns on: each tick in the dialog ticked (an excess agreed
/// to, a large spend, «I understand»), each empty field of the payment typed ([`filler`]). The
/// choices (a key's purpose, a rule) are tried one by one by the test itself.
pub(super) fn fill_the_dialog(kit: &mut Kit, rect: egui::Rect) {
    let inside = |r: egui::accesskit::Rect| rect.contains(centre(r));
    let fields: Vec<(egui::Pos2, &'static str)> = kit
        .query_all_by_role(Role::TextInput)
        .filter(|b| b.accesskit_node().bounding_box().is_some_and(inside))
        .filter(|b| b.accesskit_node().value().is_none_or(|v| v.is_empty()))
        .filter_map(|b| {
            let n = b.accesskit_node();
            Some((
                centre(n.bounding_box()?),
                filler(&n.label().unwrap_or_default())?,
            ))
        })
        .collect();
    for (at, text) in fields {
        click_at(kit, at);
        kit.event(egui::Event::Text(text.into()));
        kit.run_steps(3);
    }
    // Ticked last: a tick agrees to what was typed, and a change takes it off.
    let ticks: Vec<egui::Pos2> = kit
        .query_all_by_role(Role::CheckBox)
        .filter(|b| b.accesskit_node().bounding_box().is_some_and(inside))
        .filter(|b| b.accesskit_node().toggled() != Some(egui::accesskit::Toggled::True))
        .filter_map(|b| b.accesskit_node().bounding_box())
        .map(centre)
        .collect();
    for at in ticks {
        click_at(kit, at);
    }
}

/// The second pass, for a dialog still off: each field of the payment given a value that pays,
/// over what the preset put there (an amount over what there is, the token's own names).
fn overwrite_the_dialog(kit: &mut Kit, rect: egui::Rect) {
    let inside = |r: egui::accesskit::Rect| rect.contains(centre(r));
    let fields: Vec<(egui::Pos2, &'static str)> = kit
        .query_all_by_role(Role::TextInput)
        .filter(|b| b.accesskit_node().bounding_box().is_some_and(inside))
        .filter_map(|b| {
            let n = b.accesskit_node();
            Some((
                centre(n.bounding_box()?),
                filler(&n.label().unwrap_or_default())?,
            ))
        })
        .collect();
    for (at, text) in fields {
        click_at(kit, at);
        kit.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        kit.run_steps(1);
        kit.event(egui::Event::Text(text.into()));
        kit.run_steps(3);
    }
}

/// The choices of the dialog (radio buttons), by where they are.
fn choices(kit: &Kit, rect: egui::Rect) -> Vec<egui::Pos2> {
    kit.query_all_by_role(Role::RadioButton)
        .filter_map(|b| b.accesskit_node().bounding_box())
        .map(centre)
        .filter(|at| rect.contains(*at))
        .collect()
}

/// How long one press may take to be answered before the next step is looked for.
const STEP: std::time::Duration = std::time::Duration::from_secs(2);

/// The label of the button drawn at `rect`.
fn label_at(kit: &Kit, rect: egui::Rect) -> String {
    let c = rect.center();
    kit.query_all_by_role(Role::Button)
        .find(|b| {
            b.accesskit_node().bounding_box().is_some_and(|r| {
                f64::from(c.x) >= r.x0
                    && f64::from(c.x) <= r.x1
                    && f64::from(c.y) >= r.y0
                    && f64::from(c.y) <= r.y1
            })
        })
        .and_then(|b| b.accesskit_node().label())
        .unwrap_or_default()
}

/// A real click (pointer events) at `at`.
pub(super) fn click_at(kit: &mut Kit, at: egui::Pos2) {
    kit.hover_at(at);
    kit.run_steps(1);
    for pressed in [true, false] {
        kit.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
    }
    kit.run_steps(3);
}

#[test]
fn a_paying_dialog_never_offers_to_pay_again_after_it_went_through() {
    let mut wrong = Vec::new();
    let mut paid = 0;
    let mut log = Vec::new();
    for preset in PRESETS {
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
        let Some(mut seen) = drawn_now(&mut kit) else {
            continue;
        };
        // A review whose pay button waits for the password: the demo takes any.
        if seen.spends
            && seen.spend_ready.is_none()
            && kit.query_all_by_role(Role::PasswordInput).next().is_some()
        {
            type_password(&mut kit, "the demo takes any other");
            settle(&mut kit);
            if let Some(now) = drawn_now(&mut kit) {
                seen = now;
            }
        }
        // TECH-paid-once-gaps: what a person fills before the act turns on.
        if seen.spends && seen.spend_ready.is_none() {
            fill_the_dialog(&mut kit, seen.rect);
            settle(&mut kit);
            if let Some(now) = drawn_now(&mut kit) {
                seen = now;
            }
        }
        // A choice still to make (a key's purpose, a rule other than the one set): each in turn,
        // until the act turns on.
        // The second pass: values that pay over the preset's, and a token's rule other than the
        // one set («It is set this way already» has no operation).
        if seen.spends && seen.spend_ready.is_none() {
            overwrite_the_dialog(&mut kit, seen.rect);
            if let Some(form) = kit.state_mut().tokens.issuer.as_mut() {
                form.choice = if form.choice == 0 { 1 } else { 0 };
            }
            fill_the_dialog(&mut kit, seen.rect);
            settle(&mut kit);
            if let Some(now) = drawn_now(&mut kit) {
                seen = now;
            }
        }
        if seen.spends && seen.spend_ready.is_none() {
            for at in choices(&kit, seen.rect) {
                click_at(&mut kit, at);
                settle(&mut kit);
                // A choice may ask for its own field or tick.
                fill_the_dialog(&mut kit, seen.rect);
                settle(&mut kit);
                if let Some(now) = drawn_now(&mut kit) {
                    seen = now;
                }
                if seen.spend_ready.is_some() {
                    break;
                }
            }
        }
        let off_by_design = DESIGNED_OFF.iter().any(|(p, _)| p == preset);
        if seen.spends && seen.spend_ready.is_none() && !off_by_design {
            wrong.push(format!(
                "{preset} ({}): a paying dialog the test does not reach — its act stays off",
                seen.name
            ));
        }
        let (Some(button), true) = (seen.spend_ready, seen.spends) else {
            let buttons: Vec<String> = kit
                .query_all_by_role(Role::Button)
                .map(|b| {
                    let n = b.accesskit_node();
                    format!(
                        "{}{}",
                        n.label().unwrap_or_default(),
                        if n.is_disabled() { " (off)" } else { "" }
                    )
                })
                .collect();
            log.push(format!(
                "{preset} ({}): spends {} ready {:?} — {}",
                seen.name,
                seen.spends,
                seen.spend_ready,
                buttons.join(" | ")
            ));
            continue;
        };
        let name = seen.name.clone();
        kit.state_mut().last_done = None;
        kit.state_mut().pretend_paid = true;
        // Up to three presses: a summary out of sight takes one first, a form's «Review» opens
        // the review — each next press only once the last one is answered or a step on.
        let mut pay = String::new();
        let mut pays = Vec::new();
        let mut at = Some(button);
        for _ in 0..3 {
            let Some(button) = at else { break };
            pay = label_at(&kit, button);
            pays.push(pay.clone());
            click_at(&mut kit, button.center());
            let started = std::time::Instant::now();
            while kit.state().last_done.is_none() && started.elapsed() < STEP {
                std::thread::sleep(std::time::Duration::from_millis(5));
                kit.step();
            }
            if kit.state().last_done.is_some() {
                break;
            }
            settle(&mut kit);
            at = drawn_now(&mut kit).and_then(|d| d.spend_ready);
            // The next step's review waits for the password.
            if at.is_none() && kit.query_all_by_role(Role::PasswordInput).next().is_some() {
                type_password(&mut kit, "the demo takes any other");
                settle(&mut kit);
                at = drawn_now(&mut kit).and_then(|d| d.spend_ready);
            }
        }
        let started = std::time::Instant::now();
        while kit.state().last_done.is_none() && started.elapsed() < PATIENCE {
            std::thread::sleep(std::time::Duration::from_millis(5));
            kit.step();
        }
        match kit.state().last_done.clone() {
            // A dialog whose only act is to close: nothing to pay here.
            None if pay == "Close" => log.push(format!("{preset} ({name}): only «Close»")),
            None => wrong.push(format!("{preset} ({name}): «{pay}» sent nothing")),
            // A paying preset that does not reach a success is a gap, not a pass (the
            // coordinator's rule after FEAT-0048): only a frame refused by design.
            Some((what, false)) if off_by_design => {
                log.push(format!("{preset} ({name}): refused {what}, by design"))
            }
            Some((what, false)) => wrong.push(format!(
                "{preset} ({name}): «{pay}» did not reach a success: refused {what}"
            )),
            Some((_, true)) => {
                paid += 1;
                log.push(format!("{preset} ({name}): «{pay}» paid"));
                settle(&mut kit);
                // The same dialog still open with a button on: pressed again, it sends nothing
                // (the card's acceptance) — «Done» or «Close» of the result closes, the form
                // QA saw («Register» on again for the name just bought) pays a second time.
                // The same dialog still open: none of the buttons that paid, on or off — the result
                // has no pay button (the card: «closes, or shows the name registered without a
                // pay button»; QA's «Register» was there, off only because of the answer).
                if let Some(after) = drawn_now(&mut kit)
                    && after.name == name
                {
                    let inside = |r: &egui::accesskit::Rect| {
                        after.rect.contains(egui::pos2(
                            ((r.x0 + r.x1) / 2.0) as f32,
                            ((r.y0 + r.y1) / 2.0) as f32,
                        ))
                    };
                    let shown: Vec<String> = kit
                        .query_all_by_role(Role::Button)
                        .filter(|b| {
                            b.accesskit_node()
                                .bounding_box()
                                .is_some_and(|r| inside(&r))
                        })
                        .filter_map(|b| b.accesskit_node().label())
                        .filter(|l| pays.contains(l))
                        .collect();
                    if !shown.is_empty() {
                        wrong.push(format!(
                            "{preset} ({name}): after «{pay}» went through, still shown: {}",
                            shown.join(", ")
                        ));
                    }
                }
                if let Some(after) = drawn_now(&mut kit)
                    && after.name == name
                    && let Some(button) = after.spend_ready
                {
                    let again = label_at(&kit, button);
                    kit.state_mut().last_done = None;
                    click_at(&mut kit, button.center());
                    let started = std::time::Instant::now();
                    while kit.state().last_done.is_none() && started.elapsed() < STEP {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        kit.step();
                    }
                    if let Some((what, _)) = kit.state().last_done.clone() {
                        wrong.push(format!(
                            "{preset} ({name}): after «{pay}» went through, «{again}» paid again ({what})"
                        ));
                    }
                }
            }
        }
    }
    // What each preset did, for `--nocapture`.
    eprintln!("{}", log.join("\n"));
    assert!(paid > 10, "only {paid} presets paid:\n{}", log.join("\n"));
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
