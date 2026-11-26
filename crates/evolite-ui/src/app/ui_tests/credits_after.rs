//! FEAT-0057 П3 on the demo core: «Credits @name: now › after» in every window that writes to
//! Platform on an identity's credits (A3.1), amber with «Top up» when few would be left (A3.2),
//! dots under «Hide amounts» (A3.4).

use super::*;

/// What the line's row says: «… credits: now › ≈ after», «…: now › at least after», or
/// «…: checking…».
fn lines(kit: &Kit) -> Vec<String> {
    kit.query_all_by(|n| n.role() == Role::Label)
        .filter_map(|n| {
            let node = n.accesskit_node();
            node.label().or_else(|| node.value()).map(|w| w.to_string())
        })
        .filter(|v| v.contains(" credits: "))
        .collect()
}

/// The line, with its numbers drawn.
fn the_line(kit: &Kit) -> Option<String> {
    lines(kit).into_iter().find(|v| {
        v.ends_with(": checking…")
            || (v.contains(" › ")
                && (v.contains("› ≈ ") || v.contains("› at least "))
                && v.ends_with("DASH"))
    })
}

/// The write windows of the card's table, the token windows too (A3.1).
const WRITE_WINDOWS: [&str; 15] = [
    "add-contact",
    "contact-accept",
    "add-name-review",
    "key-add",
    "key-disable",
    "edit-profile",
    "connect",
    "apps-disconnect",
    "workshop-review",
    "workshop-model-review",
    "workshop-give",
    "tokens-send",
    "tokens-claim",
    "tokens-buy",
    "tokens-mint",
];

/// `preset`'s window, all of it in view (a summary may sit below a long body), charging.
fn write_window(preset: &str) -> (std::sync::MutexGuard<'static, ()>, Kit) {
    let (turn, mut kit) = demo_app(preset, "en");
    kit.set_size(egui::vec2(1280.0, 1600.0));
    // «Add a key» charges once a purpose is chosen (K5): transfers, which this identity lacks.
    if preset == "key-add"
        && let Some(c) = kit.state_mut().identity_keys.change.as_mut()
    {
        c.chosen = Some(evolite_api::key_plan::KeyRequest::Transfers);
    }
    // BUG-298: key 1 is the last sign-in key — its warning first; the summary, with the line,
    // once «Disable anyway» is chosen.
    if preset == "key-disable"
        && let Some(c) = kit.state_mut().identity_keys.change.as_mut()
    {
        c.anyway = true;
    }
    (turn, kit)
}

/// A3.1: every write window of the card's table, the token windows too, has the line.
#[test]
fn every_write_window_says_the_credits_now_and_after() {
    for preset in WRITE_WINDOWS {
        let (_turn, mut kit) = write_window(preset);
        wait_for(&mut kit, preset, |kit| the_line(kit).is_some());
    }
}

/// §8d, the class (r2, Must 2): in every write window a failed read is said — «(last known) ·
/// could not update» or «could not read» — with no «after»; none feeds the line by itself.
#[test]
fn every_write_window_says_a_failed_credits_read() {
    let mut wrong = Vec::new();
    for preset in WRITE_WINDOWS {
        for read in [true, false] {
            let (_turn, mut kit) = write_window(preset);
            wait_for(&mut kit, preset, |kit| the_line(kit).is_some());
            let words = if read {
                "(last known) · could not update"
            } else {
                "could not read"
            };
            for _ in 0..40 {
                if lines(&kit).iter().any(|v| v.ends_with(words)) {
                    break;
                }
                show_wallet_edited(
                    &mut kit,
                    2,
                    words,
                    move |w| {
                        w.balances.credits_failed = true;
                        w.balances.credits_read = read;
                    },
                    |_| true,
                );
            }
            let shown = lines(&kit);
            if !shown.iter().any(|v| v.ends_with(words)) || shown.iter().any(|v| v.contains(" › "))
            {
                wrong.push(format!("{preset}, read {read}: {shown:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A3.2: credits for a write and a half — after it, less than another: amber, «Few credits will
/// be left» and «Top up», which opens the identity's top-up.
#[test]
fn few_credits_left_is_said_with_a_top_up() {
    let (_turn, mut kit) = demo_app("add-contact", "en");
    kit.set_size(egui::vec2(1280.0, 1600.0));
    wait_for(&mut kit, "the line", |kit| the_line(kit).is_some());
    let fee = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.fees.contact_request)
        .expect("the demo wallet");
    show_wallet_edited(
        &mut kit,
        2,
        "few left",
        |w| w.balances.credits = fee * 3 / 2,
        |kit| has_text(kit, "Few credits will be left"),
    );
    kit.query_all_by_label("Top up")
        .next()
        .expect("«Top up»")
        .click();
    wait_for(&mut kit, "the top-up", |kit| {
        kit.state().screen == crate::screens::Screen::Identity
            && kit.state().identity.top_up.is_some()
    });
}

/// A3.4: «Hide amounts» — dots for the credits, now and after.
#[test]
fn hidden_amounts_hide_the_credits_line() {
    let (_turn, mut kit) = demo_app("add-contact", "en");
    kit.set_size(egui::vec2(1280.0, 1600.0));
    kit.state_mut().settings.set("hide-amounts", true);
    wait_for(&mut kit, "the line, hidden", |kit| {
        lines(kit)
            .iter()
            .any(|v| v.contains(" › ") && v.contains('•'))
    });
}

/// §8d (the architect): a failed read — its last known amount with «could not update», or with
/// none «could not read»; no «after» either way; at 1024 and 800 × 440.
#[test]
fn a_failed_credits_read_is_said_never_as_a_zero() {
    for size in [egui::vec2(1024.0, 768.0), egui::vec2(800.0, 440.0)] {
        for read in [true, false] {
            let (_turn, mut kit) = demo_app("add-contact", "en");
            kit.set_size(size);
            let words = if read {
                "(last known) · could not update"
            } else {
                "could not read"
            };
            show_wallet_edited(
                &mut kit,
                2,
                words,
                move |w| {
                    w.balances.credits_failed = true;
                    w.balances.credits_read = read;
                },
                |kit| has_words(kit, words),
            );
            let shown = lines(&kit);
            assert!(
                shown.iter().any(|v| v.ends_with(words))
                    && !shown.iter().any(|v| v.contains(" › ")),
                "{size:?}, read {read}: {shown:?}, no «after»"
            );
        }
    }
}
