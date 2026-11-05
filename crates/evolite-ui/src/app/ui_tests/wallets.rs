//! Branch C: another wallet deleted in place (BUG-44), the screens of one wallet not shown in
//! another (BUG-45), "Switching…" while the core answers (BUG-39 №15).

use super::*;
use crate::app::WalletBusy;
use crate::screens::{self, Screen};
use evolite_api::{Command, CoreError, Password, VaultState};

/// The demo's second wallet ("Savings"), not open at start.
const SAVINGS: &str = "5e6f7a8b";

fn open_wallet(kit: &Kit) -> Option<String> {
    kit.state()
        .snapshot
        .as_ref()?
        .wallets
        .iter()
        .find(|w| w.open)
        .map(|w| w.id.clone())
}

/// Clicks a field, so that it takes the typing, and types `text` into it.
fn type_into(kit: &mut Kit, role: Role, nth_from_end: usize, text: &str) {
    kit.query_all_by_role(role)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .nth(nth_from_end)
        .expect("the field")
        .click();
    kit.run_steps(2);
    kit.query_all_by_role(role)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .nth(nth_from_end)
        .expect("the field")
        .type_text(text);
    kit.run_steps(2);
}

/// BUG-44 (R-28): Settings → Wallets → "…" of a wallet that is not open → "Delete from this
/// computer" opens the deletion right there, by that wallet's password; the open wallet stays
/// open and unlocked, and the other one leaves the list.
#[test]
fn another_wallet_is_deleted_in_place_and_the_open_one_stays() {
    let (_turn, mut kit) = demo_app("wallet-more", "en");
    wait_for(&mut kit, "the wallet's menu", |kit| {
        kit.query_by_label("Delete from this computer").is_some()
    });
    button(&kit, "Delete from this computer").click();
    kit.run_steps(3);
    let other = kit
        .state()
        .settings
        .delete
        .as_ref()
        .map(|f| f.other.clone());
    assert_eq!(
        other,
        Some(Some((SAVINGS.to_owned(), "Savings".to_owned()))),
        "the deletion is Savings', here in Settings"
    );
    assert!(has_text(&kit, "Password of “Savings”"));
    assert_eq!(
        open_wallet(&kit).as_deref(),
        Some("a1b2c3d4"),
        "Main stays open"
    );

    type_into(&mut kit, Role::PasswordInput, 0, "savings-password");
    type_into(&mut kit, Role::TextInput, 0, "delete");
    // On its box: a click on its words does not tick it in kittest (the label takes it).
    let rect = kit
        .get_by_label("The phrase is written down and checked")
        .rect();
    click_at(&mut kit, egui::pos2(rect.left() + 10.0, rect.center().y));
    let words: Vec<String> = kit
        .query_all_by_role(Role::TextInput)
        .map(|n| format!("{:?}", n.accesskit_node().value()))
        .chain(
            kit.query_all_by_role(Role::PasswordInput)
                .map(|n| format!("pw {:?}", n.accesskit_node().value())),
        )
        .chain(
            kit.query_all_by_role(Role::CheckBox)
                .map(|n| format!("cb {:?}", n.accesskit_node().toggled())),
        )
        .collect();
    assert!(
        !button(&kit, "Delete from the computer")
            .accesskit_node()
            .is_disabled(),
        "everything asked is there: {words:?}"
    );
    button(&kit, "Delete from the computer").click();
    kit.run_steps(2);
    wait_for(&mut kit, "Savings gone from the list", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.wallets.iter().any(|w| w.id == SAVINGS))
    });
    kit.run_steps(3);
    assert!(kit.state().settings.delete.is_none(), "the dialog closes");
    let snapshot = kit.state().snapshot.clone().unwrap();
    assert_eq!(snapshot.vault, VaultState::Unlocked, "Main is not locked");
    assert_eq!(open_wallet(&kit).as_deref(), Some("a1b2c3d4"));
    assert_eq!(kit.state().screen, Screen::Settings);
}

/// BUG-44: the other wallet's deletion is answered as "delete-other": a wrong password stays
/// in its dialog, and the open wallet's forms do not take it for their own deletion.
#[test]
fn a_wrong_password_for_another_wallet_keeps_its_dialog() {
    let (_turn, mut kit) = demo_app("wallet-more", "en");
    wait_for(&mut kit, "the wallet's menu", |kit| {
        kit.query_by_label("Delete from this computer").is_some()
    });
    button(&kit, "Delete from this computer").click();
    kit.run_steps(3);
    kit.state_mut().core.send(Command::DeleteWallet {
        password: Some(Password::new(String::new())),
        wallet: Some(SAVINGS.into()),
    });
    wait_for(&mut kit, "«Wrong password»", |kit| {
        has_text(kit, "Wrong password")
    });
    assert!(kit.state().settings.delete.is_some());
    assert!(
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.wallets.iter().any(|w| w.id == SAVINGS))
    );
    assert_eq!(kit.state().screen, Screen::Settings);
}

/// BUG-45: Main's "Credits added" and its saved CSV are not shown once Savings is open.
#[test]
fn another_wallet_opens_without_the_first_ones_banners() {
    let (_turn, mut kit) = demo_app("overview", "en");
    wait_for(&mut kit, "Main open", |kit| {
        open_wallet(kit).as_deref() == Some("a1b2c3d4")
    });
    kit.state_mut().identity.top_up_result = Some((1_000_000, Ok(())));
    kit.state_mut().contacts.result = Some(Err(CoreError::WrongPassword));
    let saved: Result<(std::path::PathBuf, usize), String> =
        Ok(("transaction-history.csv".into(), 6));
    kit.ctx
        .data_mut(|d| d.insert_temp(egui::Id::new("csv-export"), saved));
    kit.run_steps(2);
    assert!(screens::activity::saved(&kit.ctx).is_some());

    kit.state_mut().core.send(Command::SwitchWallet {
        id: SAVINGS.into(),
        password: Some(Password::new("savings-password".into())),
    });
    wait_for(&mut kit, "Savings open", |kit| {
        open_wallet(kit).as_deref() == Some(SAVINGS)
    });
    kit.run_steps(2);
    assert!(
        kit.state().identity.top_up_result.is_none(),
        "Main's top-up"
    );
    assert!(kit.state().contacts.result.is_none());
    assert!(screens::activity::saved(&kit.ctx).is_none(), "Main's CSV");
    // A late answer to Main's top-up is not Savings' either.
    kit.state_mut().identity.on_top_up_done(&Ok(()));
    assert!(kit.state().identity.top_up_result.is_none());
}

/// BUG-39 №15: "Add wallet" shows "Switching…" from the click until the app has the core's
/// answer, and not for good.
#[test]
fn switching_shows_from_add_wallet_until_the_answer() {
    let (_turn, mut kit) = demo_app("wallet-menu", "en");
    // While no answer has come (none asked of the demo core here), it is on screen.
    kit.state_mut().adding = Some(None);
    kit.state_mut().wallet_busy = Some((std::time::Instant::now(), WalletBusy::Adding));
    kit.run_steps(2);
    assert!(has_text(&kit, "Switching…"));
    // Never for good.
    kit.state_mut().wallet_busy = Some((
        std::time::Instant::now() - crate::app::WALLET_BUSY_FOR,
        WalletBusy::Adding,
    ));
    kit.run_steps(2);
    assert!(!has_text(&kit, "Switching…"));
    assert!(kit.state().wallet_busy.is_none());
    kit.state_mut().adding = None;
    kit.run_steps(2);

    button(&kit, "Add wallet").click();
    kit.run_steps(1);
    assert!(
        matches!(kit.state().wallet_busy, Some((_, WalletBusy::Adding))),
        "set by the click itself"
    );
    wait_for(&mut kit, "room for the new wallet", |kit| {
        kit.state().adding.is_none() && kit.state().screen == Screen::Welcome
    });
    kit.run_steps(2);
    assert!(kit.state().wallet_busy.is_none(), "gone with the answer");
    assert!(!has_text(&kit, "Switching…"));
}

/// BUG-39 №15: "‹ Back to “Main”" shows "Switching…" in its place from the click on, until
/// the core has stopped adding.
#[test]
fn switching_shows_from_back_to_until_the_core_is_back() {
    let (_turn, mut kit) = demo_app("wallet-menu", "en");
    button(&kit, "Add wallet").click();
    kit.run_steps(2);
    wait_for(&mut kit, "room for the new wallet", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.adding_wallet)
            && kit.state().screen == Screen::Welcome
    });
    kit.run_steps(2);
    kit.get_by_label("Back to “Main”").click();
    kit.run_steps(1);
    assert!(
        matches!(kit.state().wallet_busy, Some((_, WalletBusy::Back))),
        "set by the click itself"
    );
    wait_for(&mut kit, "way back to “Main”", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.adding_wallet)
    });
    kit.run_steps(2);
    assert!(kit.state().wallet_busy.is_none());
    assert!(!has_text(&kit, "Switching…"));
}

/// A click of the pointer at `pos`, as a person makes it.
fn click_at(kit: &mut Kit, pos: egui::Pos2) {
    kit.event(egui::Event::PointerMoved(pos));
    kit.run_steps(1);
    for pressed in [true, false] {
        kit.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        kit.run_steps(1);
    }
    kit.run_steps(1);
}
