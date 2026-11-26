//! FEAT-0023 round 4: App Connect v2 on screen, on the demo core (docs/design/
//! app-connect-v2-2026-11-11.md): the request, the list's states, renewing, removing and the
//! refusals.

use super::*;
use evolite_api::Command;

/// The demo's Yappr request, opened as a pasted link would open it.
fn open_request(kit: &mut Kit) {
    kit.state_mut().core.send(Command::OpenAppLink {
        text: "dashpay://connect?v=2&n=t&r=AQ".into(),
    });
    wait_for(kit, "the request dialog", |kit| {
        kit.state().requests.open.is_some()
    });
    kit.run_steps(3);
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

fn enabled(kit: &Kit, name: &str) -> bool {
    kit.query_all_by_role_and_label(Role::Button, name)
        .last()
        .is_some_and(|b| !b.accesskit_node().is_disabled())
}

/// AC-D1–D3: the request says who the app is in its own words, what it may and may not do, the
/// limit and period it asks (selected), the warning about older apps before the password, and
/// what it costs; «Connect» waits for the password, which is asked even with the setting off.
#[test]
fn the_request_says_what_the_app_gets_and_waits_for_the_password() {
    let (_turn, mut kit) = demo_app("apps", "en");
    assert!(
        !kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .unwrap()
            .password_each_send,
        "the setting is off in the demo"
    );
    open_request(&mut kit);
    for words in [
        "Connect an app",
        "what the app calls itself",
        "The app will be able to",
        "Write to the Yappr contracts in your name",
        "Change your DashPay profile",
        "Transfer credits and DASH",
        "See the phrase and other keys",
        "Spending limit, DASH",
        "This is permanent for",
        "Adding keys",
    ] {
        assert!(has_words(&kit, words), "«{words}»");
    }
    assert!(
        !has_words(&kit, "I understand"),
        "no checkbox before the password"
    );
    assert!(!enabled(&kit, "Connect"), "not before the password");
    assert!(has_text(&kit, "Enter the password"));
    type_password(&mut kit, "any");
    assert!(enabled(&kit, "Connect"));
}

/// A1, after: «Connect» sends the choices; the list says the app is connected and shows it.
#[test]
fn connecting_puts_the_app_in_the_list() {
    let (_turn, mut kit) = demo_app("apps", "en");
    show_wallet_edited(&mut kit, 2, "no apps", |w| w.apps.clear(), |_| true);
    open_request(&mut kit);
    type_password(&mut kit, "any");
    button(&kit, "Connect").click();
    wait_for(&mut kit, "the app connected", |kit| {
        kit.state().requests.connected.is_some()
    });
    kit.run_steps(3);
    assert!(kit.state().requests.open.is_none(), "the dialog closed");
    assert!(has_words(&kit, "Yappr connected: limit 0.51"), "the banner");
    assert!(has_text(&kit, "Active"));
    assert!(has_words(&kit, "Spent"));
}

/// The optional key can be left off: the grant has none for it, the primary key all the limit.
#[test]
fn an_optional_key_left_off_is_not_granted() {
    let (_turn, mut kit) = demo_app("apps", "en");
    open_request(&mut kit);
    // By AccessKit, as a reader: the summary over the password grows with each encryption key
    // the app gets (the demo's own), and at the test window's height the box can be below the
    // fold of the middle, where a click at its centre would miss it.
    kit.query_by_role_and_label(Role::CheckBox, "Change your DashPay profile")
        .expect("the optional key's checkbox")
        .click_accesskit();
    kit.run_steps(2);
    let state = &kit.state().requests;
    let request = state.open.clone().unwrap();
    let grant =
        crate::screens::connect2::grant(&request, &state.choice, chrono::Local::now().timestamp())
            .expect("a grant");
    assert_eq!(grant.keys[1], None);
    assert_eq!(grant.keys[0].map(|(b, _)| b), Some(51_000_000));
}

/// An encryption key the app gets with none added — the identity's own current decryption key of
/// another contract — is said in the request and again in the summary over the password, as the
/// private key the app receives; a key the wallet does not send is not said.
#[test]
fn a_key_the_app_gets_without_adding_one_is_said() {
    const OTHER: &str = "9Mz3Rt6wQmRz8LpXcV4nB7sYd3Ke1JaPu5W2x";
    let (_turn, mut kit) = demo_app("apps", "en");
    // Tall enough for the whole middle: AccessKit keeps a widget's whole box, clipped or not,
    // so a middle that scrolls would put its own copy of a line inside the summary's place.
    kit.set_size(egui::vec2(1024.0, 2000.0));
    open_request(&mut kit);
    let key = "Get your private decryption key for another contract";
    // Each line drawn: its label's node, not its text run too — AccessKit gives a painted line
    // a `TextRun` child with the same text and box, and counting both saw each line twice
    // (audit F1: «in the summary», 2 for 1).
    let lines = |kit: &Kit| -> Vec<egui::Rect> {
        kit.query_all_by(|n| {
            n.role() != Role::TextRun
                && (n.label().is_some_and(|l| l.contains(key))
                    || n.value().is_some_and(|v| v.contains(key)))
        })
        .map(|n| n.rect())
        .collect()
    };
    for given in [true, false] {
        if let Some(open) = kit.state_mut().requests.open.as_mut() {
            let evolite_api::AppRequestKind::Connect2 { encryption, .. } = &mut open.kind;
            *encryption = vec![evolite_api::EncryptionOffer {
                contract: OTHER.into(),
                encryption: false,
                given,
                added: false,
            }];
        }
        kit.run_steps(3);
        let seen = crate::components::dialog_seen(&kit.ctx).expect("the request dialog");
        assert!(
            seen.scrolled.is_none() && !seen.one_scroll,
            "given {given}: the middle fits whole, the summary pinned under it"
        );
        let (summary, _) = seen.summary.expect("the request's summary");
        let shown = lines(&kit);
        let pinned = shown
            .iter()
            .filter(|r| summary.expand(0.5).contains_rect(**r))
            .count();
        assert_eq!(pinned, usize::from(given), "given {given}: in the summary");
        assert_eq!(
            shown.len() - pinned,
            usize::from(given),
            "given {given}: in the request"
        );
        for words in [
            "receives the private key itself",
            "already has for this contract",
            "This is not the app's own contract",
        ] {
            assert_eq!(has_words(&kit, words), given, "given {given}: «{words}»");
        }
    }
}

/// L1/L2: active, used up (with what to do), expired, and the disconnected ones apart with
/// «Remove from the list» only; never red.
#[test]
fn the_list_shows_each_connections_state() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.run_steps(3);
    for words in [
        "Connected · 3",
        "Active",
        "Limit used up",
        "Expired on",
        "The app can no longer write anything in your name.",
        "Disconnected · 1",
        "disconnected on",
    ] {
        assert!(has_words(&kit, words), "«{words}»");
    }
    assert_eq!(
        kit.query_all_by_role_and_label(Role::Button, "Remove from the list")
            .count(),
        1,
        "only the disconnected one"
    );
    assert_eq!(
        kit.query_all_by_role_and_label(Role::Button, "Renew")
            .count(),
        3
    );
}

/// D2: removing a disconnected app asks once, with no password, and it leaves the list.
#[test]
fn a_disconnected_app_leaves_the_list_without_a_password() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.run_steps(3);
    // The fourth card, below the fold at the test window's height: by AccessKit, as a reader.
    button(&kit, "Remove from the list").click_accesskit();
    kit.run_steps(3);
    assert!(has_text(&kit, "Remove from the list?"));
    assert!(kit.query_all_by_role(Role::PasswordInput).next().is_none());
    kit.query_all_by_role_and_label(Role::Button, "Remove from the list")
        .last()
        .expect("the dialog's button")
        .click();
    wait_for(&mut kit, "the app removed", |kit| {
        !has_words(kit, "Disconnected · 1")
    });
}

/// R1: «Renew» says a new sign-in renews it, with the limit as it stands, and offers the link.
#[test]
fn renewing_explains_the_new_sign_in() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.run_steps(3);
    kit.query_all_by_role_and_label(Role::Button, "Renew")
        .next()
        .expect("«Renew»")
        .click();
    kit.run_steps(3);
    assert!(has_text(&kit, "Renew the Yappr connection"));
    assert!(has_words(&kit, "it takes a new sign-in"));
    kit.query_all_by_role_and_label(Role::Button, "Paste a link")
        .last()
        .expect("«Paste a link»")
        .click();
    kit.run_steps(3);
    assert!(kit.state().requests.pasting, "the paste dialog opens");
}

/// E1: a v1 link is refused in its own dialog, with its version and that nothing was spent.
#[test]
fn a_v1_link_is_refused_with_its_version() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.state_mut().core.send(Command::OpenAppLink {
        text: "dash-key:abc?n=t&v=1".into(),
    });
    wait_for(&mut kit, "the refusal", |kit| {
        kit.state().requests.refused.is_some()
    });
    kit.run_steps(3);
    assert!(has_text(&kit, "Cannot connect"));
    assert!(has_words(&kit, "(v=1)"));
    assert!(has_words(&kit, "Nothing was spent."));
    button(&kit, "Got it").click();
    kit.run_steps(2);
    assert!(kit.state().requests.refused.is_none());
}

/// The rect of the first node whose label or value starts with `words`.
fn rect_of(kit: &Kit, words: &str) -> egui::Rect {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.starts_with(words))
            || n.value().is_some_and(|v| v.starts_with(words))
    })
    .next()
    .unwrap_or_else(|| panic!("«{words}»"))
    .rect()
}

/// The room between the line starting with `under` and the nearest title above it starting with
/// `title` (the list behind the dialog has the same titles).
fn gap_above(kit: &Kit, under: &str, title: &str) -> f32 {
    let line = rect_of(kit, under);
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.starts_with(title))
            || n.value().is_some_and(|v| v.starts_with(title))
    })
    .map(|n| n.rect())
    .filter(|r| r.bottom() <= line.top() + 0.5 && (r.left() - line.left()).abs() < 60.0)
    .map(|r| line.top() - r.bottom())
    .min_by(f32::total_cmp)
    .unwrap_or_else(|| panic!("«{title}» above «{under}»"))
}

/// A1, the designer's r4 note 3: the line under an optional key (with its checkbox) stands as
/// close to its title as the line under the primary key does.
#[test]
fn the_line_under_each_key_is_as_close_to_its_title() {
    let (_turn, mut kit) = demo_app("apps", "en");
    open_request(&mut kit);
    // FEAT-0051 UC-4: the contract and its owner whole make the body taller; both keys
    // measured in sight, not clipped by the fold.
    kit.query_by_role_and_label(Role::CheckBox, "Change your DashPay profile")
        .expect("the optional key's checkbox")
        .scroll_to_me();
    kit.run_steps(3);
    let primary = gap_above(&kit, "fees come from", "Write to");
    let optional = gap_above(&kit, "up to ", "Change your DashPay profile");
    assert!(
        (primary - optional).abs() <= 2.0,
        "under the primary key {primary} px, under the optional one {optional} px"
    );
}

/// L1, the designer's r4 note 2: a state chip has its icon before its words, as every status
/// in the app (the card's right-to-left row must not turn it round).
#[test]
fn a_state_chip_has_its_icon_before_its_words() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.run_steps(3);
    let words = rect_of(&kit, "Active");
    let icon = kit
        .query_all_by(|n| n.role() == Role::Image)
        .map(|n| n.rect())
        .find(|r| {
            (r.center().y - words.center().y).abs() < 6.0 && (r.left() - words.right()).abs() < 30.0
                || (r.center().y - words.center().y).abs() < 6.0
                    && (words.left() - r.right()).abs() < 30.0
        })
        .expect("the chip's icon beside «Active»");
    assert!(
        icon.right() <= words.left(),
        "the icon at {icon:?}, the words at {words:?}"
    );
}

/// BUG-147 AC-T1: «Apps» asks for the connected apps' budgets on each visit, not once a run:
/// opened, left and opened again, it asks again. While it stays open it asks again only after
/// 30 s (the screen's own test of `budgets_due`).
#[test]
fn each_visit_of_apps_reads_the_budgets_again() {
    let (_turn, mut kit) = demo_app("apps", "en");
    wait_for(&mut kit, "the budgets asked on the first visit", |kit| {
        !kit.state().requests.budgets_asked.is_empty()
    });
    kit.state_mut().screen = crate::screens::Screen::Overview;
    kit.run_steps(2);
    assert!(
        kit.state().requests.budgets_asked.is_empty(),
        "leaving «Apps» forgets what was asked"
    );
    kit.state_mut().screen = crate::screens::Screen::Apps;
    wait_for(&mut kit, "the budgets asked on the second visit", |kit| {
        !kit.state().requests.budgets_asked.is_empty()
    });
}

/// The lines drawn with `words` in them: a label's node, not its text run too (audit F1).
fn drawn(kit: &Kit, words: &str) -> Vec<egui::Rect> {
    kit.query_all_by(|n| {
        n.role() != Role::TextRun
            && (n.label().is_some_and(|l| l.contains(words))
                || n.value().is_some_and(|v| v.contains(words)))
    })
    .map(|n| n.rect())
    .collect()
}

/// The open request with `keys` encryption keys the app gets (none added) and V1's warning or not.
fn with_keys(kit: &mut Kit, keys: usize, v1: bool) {
    const OTHERS: [&str; 3] = [
        "9Mz3Rt6wQmRz8LpXcV4nB7sYd3Ke1JaPu5W2x",
        "7Qw2Er5tYuIoPaSdFgHjKlZxCvBnM1234567a",
        "4Ab5Cd6Ef7Gh8Jk9Lm1Np2Qr3St4Uv5Wx6Yz7b",
    ];
    if let Some(open) = kit.state_mut().requests.open.as_mut() {
        let evolite_api::AppRequestKind::Connect2 {
            encryption,
            breaks_old_clients,
            ..
        } = &mut open.kind;
        *encryption = OTHERS[..keys]
            .iter()
            .map(|c| evolite_api::EncryptionOffer {
                contract: (*c).into(),
                encryption: false,
                given: true,
                added: false,
            })
            .collect();
        *breaks_old_clients = v1;
    }
}

/// FEAT-0051 r2 (the architect): V1's «Irreversible» in one amber line in the pinned summary,
/// once, over the key lines; the whole warning stays in the request.
#[test]
fn the_irreversible_line_is_pinned_once_over_the_keys() {
    let (_turn, mut kit) = demo_app("apps", "en");
    // Tall enough for the whole middle (see `a_key_the_app_gets_without_adding_one_is_said`).
    kit.set_size(egui::vec2(1024.0, 2000.0));
    open_request(&mut kit);
    with_keys(&mut kit, 1, true);
    kit.run_steps(4);
    let seen = crate::components::dialog_seen(&kit.ctx).expect("the request dialog");
    assert!(!seen.one_scroll, "the summary pinned");
    let (summary, _) = seen.summary.expect("the request's summary");
    let irreversible = drawn(&kit, "Irreversible: @");
    let pinned: Vec<_> = irreversible
        .iter()
        .filter(|r| summary.expand(0.5).contains_rect(**r))
        .collect();
    assert_eq!(pinned.len(), 1, "once in the summary: {irreversible:?}");
    assert_eq!(irreversible.len(), 1, "only in the summary");
    assert!(
        has_words(&kit, "This is permanent for"),
        "the whole warning in the request"
    );
    let key = drawn(&kit, "Get your private decryption key for another contract")
        .into_iter()
        .find(|r| summary.expand(0.5).contains_rect(*r))
        .expect("the key's line in the summary");
    assert!(pinned[0].bottom() <= key.top() + 0.5, "over the key lines");
}

/// FEAT-0051 r3 (the designer, the architect: F1), BUG-249's one scroll in App Connect at
/// 800 × 440: the amounts scroll, one amber line stays with the password — the key the app gets
/// always in it (with V1, «irreversible» beside it); keys counted when there are several. A name
/// too long is shortened, never what is given or «irreversible» (ru, de). The password and the
/// buttons in the window, the middle at least 96 high.
#[test]
fn a_low_window_keeps_one_line_of_consent_with_the_password() {
    const LONG: &str = "Yappr Social Network Extended Edition For Everyone Everywhere";
    for (keys, v1, lang, long, words) in [
        (
            1,
            false,
            "en",
            false,
            &["Gives your private encryption key for Yappr"][..],
        ),
        (
            3,
            false,
            "en",
            false,
            &["Gives your private encryption keys for 3 contracts"][..],
        ),
        (
            1,
            true,
            "en",
            false,
            &[
                "Gives your private encryption key for",
                "irreversible for @satoshi",
            ][..],
        ),
        (
            1,
            true,
            "ru",
            true,
            &["закрытый ключ шифрования", "необратимо для"][..],
        ),
        (
            1,
            true,
            "de",
            true,
            &["privaten Verschlüsselungsschlüssel", "unumkehrbar für"][..],
        ),
    ] {
        let (_turn, mut kit) = demo_app("apps", lang);
        open_request(&mut kit);
        with_keys(&mut kit, keys, v1);
        if long && let Some(open) = kit.state_mut().requests.open.as_mut() {
            let evolite_api::AppRequestKind::Connect2 { label, .. } = &mut open.kind;
            *label = LONG.into();
        }
        kit.set_size(egui::vec2(800.0, 440.0));
        kit.run_steps(6);
        let case = format!("{keys} keys, V1 {v1}, {lang}");
        let seen = crate::components::dialog_seen(&kit.ctx).expect("the request dialog");
        assert!(seen.one_scroll, "{case}: the summary scrolls");
        let footer = seen.footer.expect("the footer");
        let pinned: Vec<String> = kit
            .query_all_by(|n| n.role() == Role::Label)
            .filter(|n| footer.expand(0.5).contains_rect(n.rect()))
            .filter_map(|n| n.value())
            .filter(|v| words.iter().all(|w| v.contains(w)))
            .collect();
        assert_eq!(pinned.len(), 1, "{case}: one line with {words:?}");
        if long {
            assert!(
                pinned[0].contains('…'),
                "{case}: the name shortened: {}",
                pinned[0]
            );
            assert!(!pinned[0].contains(LONG), "{case}: {}", pinned[0]);
        }
        let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 440.0));
        let password = kit
            .query_all_by_role(Role::PasswordInput)
            .next()
            .expect("the password field")
            .rect();
        assert!(window.contains_rect(password), "{case}: the password");
        let buttons: Vec<_> = kit
            .query_all_by_role(Role::Button)
            .filter(|b| footer.expand(0.5).contains_rect(b.rect()))
            .map(|b| b.rect())
            .collect();
        assert!(buttons.len() >= 2, "{case}: the buttons in the footer");
        assert!(buttons.iter().all(|b| window.contains_rect(*b)), "{case}");
        let middle = seen.scrolled.expect("the middle scrolls");
        assert!(
            middle.height() >= 96.0,
            "{case}: the middle {} high",
            middle.height()
        );
    }
}

/// BUG-323: a failed read is its own dialog — which read, «Details» with what Platform answered,
/// and «Try again», which opens the same link again (here the demo's request opens).
#[test]
fn a_failed_read_says_which_and_opens_the_link_again() {
    let (_turn, mut kit) = demo_app("apps", "en");
    kit.state_mut().requests.refused = Some(evolite_api::AppLinkRefusal::ReadFailed {
        stage: evolite_api::ConnectRead::Contract,
        undecoded: true,
        id: "8mjmcXmfLyFAX6BkawQbiuWUaNXyN3T7cSKTdTXhpDVU".into(),
        details: "protocol error: unknown version of the contract".into(),
        link: "dashpay://connect?v=2&n=t&r=AQ".into(),
    });
    kit.run_steps(3);
    assert!(has_words(
        &kit,
        "it may be newer than this version of evolite"
    ));
    assert!(
        !has_words(&kit, "unknown version of the contract"),
        "under «Details»"
    );
    button(&kit, "Details").click();
    kit.run_steps(3);
    assert!(
        has_words(&kit, "unknown version of the contract"),
        "the error's text"
    );
    button(&kit, "Try again").click();
    wait_for(&mut kit, "the request opened again", |kit| {
        kit.state().requests.open.is_some()
    });
    assert!(kit.state().requests.refused.is_none());
}
