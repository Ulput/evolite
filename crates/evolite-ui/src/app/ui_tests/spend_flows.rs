//! TECH-spend-flow AC-T2: every paid window's flow is in App's one list of what forgets, and
//! the lock, a wallet switch and an identity switch leave each of them waiting for nothing,
//! with no password typed.

use super::*;
use crate::app::{forget_screens, screen_states};
use crate::components::spend_flow;
use crate::screen_state::Why;

/// Opens every paid window that lives in a form, so that its flow is there to be forgotten.
fn open_every_form(app: &mut App) {
    app.identity.adding_name = Some(Default::default());
    app.identity.editing = Some(Default::default());
    app.tokens.sending = Some(crate::screens::tokens::SendState::new("T"));
    app.tokens.claiming = Some(crate::screens::tokens::ClaimState::new("T"));
    app.tokens.buying = Some(crate::screens::tokens::BuyState::new("T"));
    app.tokens.issuer = Some(crate::screens::tokens::manage::IssuerForm::new(
        "T",
        crate::screens::tokens::manage::IssuerWindow::Mint,
    ));
    app.identity_keys.open_enable_dashpay();
}

/// Every flow in the list as if sent, with a password typed; how many.
fn all_waiting(app: &mut App) -> usize {
    let mut n = 0;
    for state in screen_states!(app) {
        for flow in state.flows() {
            flow.wait_with_password();
            n += 1;
        }
    }
    n
}

/// The flows in the list that still wait or keep a password.
fn left(app: &mut App) -> usize {
    let mut n = 0;
    for state in screen_states!(app) {
        for flow in state.flows() {
            if !flow.is_idle() || !flow.password_empty() {
                n += 1;
            }
        }
    }
    n
}

/// AC-T2: the flows made when App is built are the flows App's list reaches — a paid
/// window's state left out of the list would never be forgotten.
#[test]
fn every_paid_window_is_in_the_one_list() {
    let _turn = ONE_APP.lock().unwrap_or_else(|e| e.into_inner());
    let args: Vec<String> = ["--screen", "overview", "--lang", "en"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let launch = Launch::from_args_with(&args, |_| None);
    let before = spend_flow::made();
    let mut kit = Harness::builder()
        .with_size(egui::vec2(1024.0, 768.0))
        .build_eframe(|cc| App::with_art(cc, launch, evolite_core::ArtOptions::off()));
    // The flows alive once App is built (a temporary made and dropped meanwhile is not one).
    let made = spend_flow::made() - before;
    let mut listed = 0;
    let app = kit.state_mut();
    for state in screen_states!(app) {
        listed += state.flows().len();
    }
    assert!(made > 0);
    assert_eq!(
        listed, made,
        "a flow made when App is built is not in the list"
    );
}

/// AC-T2: a lock (the real one, through the core), another wallet and another identity leave
/// every flow waiting for nothing and every password empty.
#[test]
fn lock_switch_and_identity_switch_forget_every_flow() {
    let (_turn, mut kit) = demo_app("overview", "en");
    open_every_form(kit.state_mut());
    let opened = all_waiting(kit.state_mut());
    assert!(opened >= 15, "every paid window open: {opened}");
    // The lock as the app sees it: the keys gone in the snapshot (App's own lock path).
    draw_edited(&mut kit, 1, |app| {
        if let Some(s) = app.snapshot.as_mut() {
            s.vault = evolite_api::VaultState::Locked;
        }
    });
    assert_eq!(left(kit.state_mut()), 0, "after the lock");

    for why in [Why::WalletSwitch, Why::IdentitySwitch] {
        open_every_form(kit.state_mut());
        all_waiting(kit.state_mut());
        let app = kit.state_mut();
        forget_screens!(app, why);
        assert_eq!(left(kit.state_mut()), 0, "after {why:?}");
    }
}
