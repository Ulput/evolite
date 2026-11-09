//! FEAT-0006: a QR from a picture — the button and its menu by the keyboard, the clipboard item
//! with its reason, an app's QR on «Apps», a name from a QR in «Add a contact», «My QR»
//! (AC-T9, AC-T11, AC-T12). The file dialog is `qr_scan::TEST_PICK` in tests; the machine's
//! clipboard is never read.

use super::*;

/// What «Choose a picture…» gives next.
fn pick(text: &str) {
    *crate::qr_scan::TEST_PICK
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(crate::qr_scan::test_png(text));
}

/// Waits for the menu and for its look at the clipboard (made off the frame).
fn menu_ready(kit: &mut Kit) {
    wait_for(kit, "the menu", |kit| {
        kit.query_by_role_and_label(Role::Button, "Choose a picture…")
            .is_some()
    });
    kit.run_steps(4);
}

/// Whether the menu is open: its items are in the tree only then.
fn menu_open(kit: &Kit) -> bool {
    kit.query_by_role_and_label(Role::Button, "Choose a picture…")
        .is_some()
}

/// The QR button in «Name in DPNS»: a Tab stop named «Scan a QR code»; Enter opens the menu
/// with the clipboard item off and its reason written (no picture: tests never read the real
/// clipboard), the focus on «Choose a picture…»; the arrows stay on it, Escape closes the
/// menu and gives the focus back to the button.
#[test]
fn the_qr_menu_works_by_the_keyboard() {
    let (_turn, mut kit) = demo_app("add-contact", "en");
    tab_to(&mut kit, "Scan a QR code");
    enter(&mut kit);
    menu_ready(&mut kit);
    kit.run_steps(2);
    let clip = kit
        .state()
        .contacts
        .adding
        .as_ref()
        .map(|a| a.scanner.clip_picture);
    assert_eq!(
        clip,
        Some(Some(false)),
        "the clipboard looked at once, no picture"
    );
    assert!(
        !kit.query_by_role_and_label(Role::Button, "From the clipboard")
            .is_some_and(|b| !b.accesskit_node().is_disabled()),
        "the clipboard item is off"
    );
    assert_eq!(focused(&kit).as_deref(), Some("Choose a picture…"));
    kit.key_press(Key::ArrowDown);
    kit.run_steps(2);
    assert_eq!(focused(&kit).as_deref(), Some("Choose a picture…"));
    kit.key_press(Key::Escape);
    kit.run_steps(2);
    assert!(!menu_open(&kit));
    assert_eq!(focused(&kit).as_deref(), Some("Scan a QR code"));
    assert!(
        kit.state().contacts.adding.is_some(),
        "Escape closed the menu, not the dialog"
    );
}

/// UC-5: a DashPay code (`dash:…?user=`) puts @name into «Name in DPNS»; a code with an
/// address only says there is no name and leaves what was typed.
#[test]
fn a_name_comes_from_a_qr_into_add_a_contact() {
    let (_turn, mut kit) = demo_app("add-contact", "en");
    let scan = |kit: &mut Kit, text: &str| {
        pick(text);
        button(kit, "Scan a QR code").click();
        kit.run_steps(2);
        menu_ready(kit);
        button(kit, "Choose a picture…").click();
        kit.run_steps(2);
        wait_for(kit, "the picture read", |kit| {
            !kit.state()
                .contacts
                .adding
                .as_ref()
                .is_some_and(|a| a.scanner.busy())
        });
        kit.run_steps(3);
    };
    scan(
        &mut kit,
        "dash:yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f?user=alice",
    );
    let adding = kit.state().contacts.adding.as_ref().expect("the dialog");
    assert_eq!(adding.name, "@alice");
    scan(&mut kit, "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f");
    assert_eq!(kit.state().contacts.adding.as_ref().unwrap().name, "@alice");
    assert!(has_text(&kit, "There is no @name in this QR code"));
}

/// UC-4, AC-T8: «Scan QR» on «Apps» is active; an app's code opens the same request as
/// «Paste a link»; a payment code there says it is not an app's request.
#[test]
fn an_apps_qr_opens_its_request() {
    let (_turn, mut kit) = demo_app("apps", "en");
    pick("dash:yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f");
    button(&kit, "Scan QR").click();
    kit.run_steps(2);
    menu_ready(&mut kit);
    button(&kit, "Choose a picture…").click();
    wait_for(&mut kit, "the answer to a payment code", |kit| {
        has_text(kit, "This is not a Dash app request")
    });
    assert!(kit.state().requests.open.is_none());
    pick("dash-key:yappr?contract=6Hk2Ft9wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q&key=MEDIUM");
    button(&kit, "Scan QR").click();
    kit.run_steps(2);
    menu_ready(&mut kit);
    button(&kit, "Choose a picture…").click();
    wait_for(&mut kit, "the app's request", |kit| {
        kit.state().requests.open.is_some()
    });
}

/// FR-8, AC-T9: «My QR» shows `dash:<the address to receive on>?user=<name>` whole, and the
/// link reads back as the same name and address.
#[test]
fn my_qr_shows_the_name_and_the_receiving_address() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    let wallet = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.clone())
        .expect("the demo wallet");
    let name = wallet
        .identity
        .as_ref()
        .and_then(|i| i.name())
        .expect("the demo identity has a name")
        .to_string();
    button(&kit, "My QR").click();
    kit.run_steps(2);
    let link = crate::payment_uri::my_link(&wallet.core_address, &name);
    assert!(has_text(&kit, &link), "«{link}» on screen");
    assert!(has_text(&kit, &format!("My QR · @{name}")));
    let read = crate::payment_uri::parse(&link).unwrap().unwrap();
    assert_eq!(read.recipient(), format!("@{name}"));
    assert_eq!(read.address, wallet.core_address);
}

/// FR-6, AC-A1: a QR of `dash:<address>?amount=…&label=…` on «Send» fills «To», «Amount» and
/// the note, and says what it filled; «Check the payment» heads the note «Note from the link».
#[test]
fn a_payment_qr_fills_send() {
    const ADDR: &str = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f";
    let (_turn, mut kit) = demo_app("send", "en");
    pick(&format!("dash:{ADDR}?amount=0.25&label=Coffee"));
    button(&kit, "Scan a QR code").click();
    kit.run_steps(2);
    menu_ready(&mut kit);
    button(&kit, "Choose a picture…").click();
    wait_for(&mut kit, "the form filled in", |kit| {
        kit.state().send.recipient == ADDR
    });
    kit.run_steps(2);
    let form = &kit.state().send;
    assert_eq!(crate::format::parse_amount(&form.amount), Some(25_000_000));
    assert_eq!(form.note, "Coffee");
    assert!(has_text(
        &kit,
        "Filled in from a QR code: address and amount. Check the address before sending"
    ));
}

/// FR-6: a `dash:` link pasted into «To» is taken as its QR is: the address alone in the field,
/// the amount in «Amount».
#[test]
fn a_dash_link_pasted_into_to_is_read() {
    const ADDR: &str = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f";
    let (_turn, mut kit) = demo_app("send", "en");
    kit.state_mut().send.recipient.clear();
    kit.run_steps(2);
    tab_to(&mut kit, "To");
    kit.event(egui::Event::Paste(format!("dash:{ADDR}?amount=1.5")));
    kit.run_steps(3);
    assert_eq!(kit.state().send.recipient, ADDR);
    assert_eq!(
        crate::format::parse_amount(&kit.state().send.amount),
        Some(150_000_000)
    );
}

/// `--screen send-qr-menu`: «Send» with «To»'s QR menu open, «From the clipboard» active; Escape
/// closes it.
#[test]
fn the_send_qr_menu_preset_opens_the_menu() {
    let (_turn, mut kit) = demo_app("send-qr-menu", "en");
    menu_ready(&mut kit);
    assert!(
        kit.query_by_role_and_label(Role::Button, "From the clipboard")
            .is_some_and(|b| !b.accesskit_node().is_disabled()),
        "the clipboard item is on"
    );
    kit.key_press(Key::Escape);
    kit.run_steps(3);
    assert!(!menu_open(&kit));
}

/// Every change of «To» is asked whether it is a `dash:` link: letters of several bytes must
/// not cut it short (this panicked), and no line appears for them.
#[test]
fn letters_that_are_not_ascii_can_be_typed_in_to() {
    let (_turn, mut kit) = demo_app("send", "en");
    kit.state_mut().send.recipient.clear();
    kit.run_steps(2);
    tab_to(&mut kit, "To");
    kit.event(egui::Event::Text("дашь".into()));
    kit.run_steps(3);
    assert_eq!(kit.state().send.recipient, "дашь");
    assert_eq!(kit.state().send.qr, None);
}

/// The line under «To» goes with what it was about, when «To» is changed by hand or otherwise.
#[test]
fn the_filled_in_line_goes_when_to_changes() {
    const WORDS: &str =
        "Filled in from a QR code: address and amount. Check the address before sending";
    let (_turn, mut kit) = demo_app("send-qr-filled", "en");
    kit.run_steps(2);
    assert!(has_text(&kit, WORDS));
    kit.state_mut().send.recipient = "@bob".into();
    kit.run_steps(2);
    assert!(!has_text(&kit, WORDS));
}

/// «Check the payment» heads a link's note as the link's and says it is not a checked name.
#[test]
fn the_last_look_heads_a_links_note() {
    let (_turn, mut kit) = demo_app("send-confirm-link", "en");
    wait_for(&mut kit, "the note from the link", |kit| {
        has_text(kit, "Note from the link")
    });
    assert!(has_text(
        &kit,
        "This is text from the QR code, not a verified recipient name."
    ));
}

fn real_receive_screenshot() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/qr/receive-real-dark.png")
}

/// A file as the window hands it to egui.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn drop_file(kit: &mut Kit, path: std::path::PathBuf) {
    kit.input_mut()
        .dropped_files
        .push(std::sync::Arc::new(Dropped(path)));
    kit.run_steps(1);
}

/// A real screenshot of Receive dropped on «Send» fills «To» with that address.
#[test]
fn a_screenshot_dropped_on_send_fills_to() {
    let (_turn, mut kit) = demo_app("send", "en");
    drop_file(&mut kit, real_receive_screenshot());
    wait_for(&mut kit, "the address from the picture", |kit| {
        kit.state().send.recipient == "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f2c"
    });
}

/// «Check the payment» is open: a picture dropped starts nothing.
#[test]
fn a_picture_dropped_during_the_last_look_is_ignored() {
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    wait_for(&mut kit, "the last look", |kit| kit.state().send.confirming);
    let before = kit.state().send.recipient.clone();
    drop_file(&mut kit, real_receive_screenshot());
    kit.run_steps(2);
    assert!(!kit.state().send.scanner.busy());
    assert_eq!(kit.state().send.recipient, before);
}

/// Another wallet becomes the open one during a scan: every screen's scan goes with its form
/// (`App` resets them), so what it reads reaches no field of the new wallet.
#[test]
fn a_wallet_switched_during_a_scan_drops_it() {
    use crate::qr_scan::Picture;
    let (_turn, mut kit) = demo_app("send", "en");
    let ctx = kit.ctx.clone();
    let code = crate::qr_scan::test_png("yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f");
    let app = kit.state_mut();
    app.send
        .scanner
        .start(&ctx, Picture::Bytes(code.clone().into()));
    app.requests
        .scanner
        .start(&ctx, Picture::Bytes(code.into()));
    app.core.send(evolite_api::Command::SwitchWallet {
        id: "5e6f7a8b".into(),
        password: Some(evolite_api::Password::new("savings-password".into())),
    });
    wait_for(&mut kit, "Savings open", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallets.iter().find(|w| w.open))
            .is_some_and(|w| w.id == "5e6f7a8b")
    });
    kit.run_steps(2);
    assert!(!kit.state().send.scanner.busy() && !kit.state().requests.scanner.busy());
    std::thread::sleep(std::time::Duration::from_millis(400));
    kit.run_steps(3);
    assert!(kit.state().send.recipient.is_empty() && kit.state().send.qr.is_none());
}
