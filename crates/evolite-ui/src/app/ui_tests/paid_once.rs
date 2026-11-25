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
fn click_at(kit: &mut Kit, at: egui::Pos2) {
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
            Some((what, false)) => log.push(format!("{preset} ({name}): refused {what}")),
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
    assert!(paid > 10, "only {paid} presets paid:\n{}", log.join("\n"));
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
