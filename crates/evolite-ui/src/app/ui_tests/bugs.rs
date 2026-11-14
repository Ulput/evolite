//! One test per bug whose UI half kittest can show: what the person does, what must follow.

use super::*;
use crate::screens::Screen;
use crate::screens::settings::NodesTab;

/// BUG-18: Settings → Network and nodes → a click on the words «Core nodes» (not on the
/// chevron) opens the nodes window on its Core tab.
#[test]
fn settings_core_nodes_opens_from_a_click_on_its_words() {
    let (_turn, mut kit) = demo_app("settings", "en");
    assert_eq!(kit.state().settings.nodes, None);
    words(&kit, "Core nodes").click();
    kit.run_steps(2);
    assert_eq!(kit.state().settings.nodes, Some(NodesTab::Core));
}

/// BUG-18 r3: the rows «Core nodes · N» and «Platform (DAPI) · N» are one Tab stop each — Tab
/// from the first lands on the second, Shift+Tab comes back — and Enter on a row opens its tab
/// of the nodes window.
///
/// Failed on dev d6cc997: Tab went «Testnet» → focus lost → the sidebar, until the row got its
/// own id (`screens/settings/parts.rs`, `row`).
#[test]
fn settings_node_rows_are_tab_stops_and_open_with_enter() {
    let (_turn, mut kit) = demo_app("settings", "en");
    tab_to(&mut kit, "Core nodes ·");
    tab(&mut kit, false);
    let next = focused(&kit).unwrap_or_default();
    assert!(
        next.starts_with("Platform (DAPI) ·"),
        "Tab went to «{next}»"
    );
    enter(&mut kit);
    assert_eq!(kit.state().settings.nodes, Some(NodesTab::Platform));

    kit.state_mut().settings.nodes = None;
    kit.run_steps(2);
    tab_to(&mut kit, "Platform (DAPI) ·");
    tab(&mut kit, true);
    let back = focused(&kit).unwrap_or_default();
    assert!(
        back.starts_with("Core nodes ·"),
        "Shift+Tab went to «{back}»"
    );
    enter(&mut kit);
    assert_eq!(kit.state().settings.nodes, Some(NodesTab::Core));
}

/// BUG-17: while a wallet is being added, a click on the words of «‹ Back to “Main”» (not on
/// the arrow) goes back to that wallet's unlock screen.
#[test]
fn back_to_the_wallet_answers_a_click_on_its_words() {
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
    click_words_of(&mut kit, "Back to “Main”");
    wait_for(&mut kit, "way back to “Main”", |kit| {
        kit.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.adding_wallet)
    });
    kit.run_steps(2);
    assert_eq!(kit.state().screen, Screen::Unlock);
    assert!(kit.query_by_label("Back to “Main”").is_none());
}

/// BUG-1: on «Which wallet to open?» in Russian the wallets are called as their owner named
/// them — «Main», «Savings» — never translated as words of the interface. (Only the names a
/// screen reader hears: the painted words are not in the tree.)
#[test]
fn wallet_names_in_the_unlock_list_are_not_translated() {
    let (_turn, mut kit) = demo_app("wallet-unlock", "ru");
    wait_for(&mut kit, "the wallets to open", |kit| {
        kit.query_by_label("Savings").is_some()
    });
    for name in ["Main", "Savings"] {
        let row = kit.get_by_role_and_label(Role::RadioButton, name);
        assert!(!row.accesskit_node().is_disabled(), "{name}");
    }
    for translated in ["Основной", "Сбережения"] {
        assert!(
            kit.query_all_by(|n| n.label().is_some_and(|l| l.contains(translated)))
                .next()
                .is_none(),
            "«{translated}» in the list"
        );
    }
}

/// BUG-20: in an operation's card «Copy txid» and «Open in the explorer» stand apart, at least
/// 12 points between them (the look of their frames is for the screenshots).
#[test]
fn the_txid_buttons_of_an_operation_card_stand_apart() {
    let (_turn, mut kit) = demo_app("activity-card", "en");
    wait_for(&mut kit, "the card's txid", |kit| {
        kit.query_by_label("Open in the explorer").is_some()
    });
    let copy = button(&kit, "Copy txid").rect();
    let open = button(&kit, "Open in the explorer").rect();
    let between = gap(copy, open);
    assert!(between >= 12.0 - 0.5, "{between} points between them");
}

/// BUG-16 (its UI half; the real core's wake is `real::tests`): a wrong password and Enter —
/// «Wrong password» comes on screen with no click or other input after the Enter.
#[test]
fn a_wrong_password_and_enter_show_the_error_at_once() {
    let (_turn, mut kit) = demo_app("wallet-unlock", "en");
    // The preset types a password; from an empty field, as a person starts.
    kit.state_mut().unlock.password.clear();
    kit.run_steps(3);
    let field = kit
        .query_all_by_role(Role::PasswordInput)
        .next()
        .expect("the password field");
    assert!(
        field.accesskit_node().is_focused(),
        "the field takes the typing"
    );
    field.type_text(evolite_core::DEMO_WRONG_PASSWORD);
    kit.run_steps(2);
    enter(&mut kit);
    wait_for(&mut kit, "«Wrong password»", |kit| {
        has_text(kit, "Wrong password")
    });
    assert_eq!(kit.state().screen, Screen::Unlock);
    assert!(kit.state().unlock.password.is_empty(), "wiped once sent");
}

/// The amount fields of a screen, as the tree has them: each field's text.
fn amounts(kit: &Kit) -> Vec<String> {
    kit.query_all_by_role(Role::TextInput)
        .filter_map(|n| n.accesskit_node().value())
        .filter(|v| crate::format::parse_amount(v).is_some())
        .collect()
}

/// An enabled button a screen reader calls `english`, in the language on screen.
fn enabled(kit: &Kit, english: &str) -> bool {
    let name = crate::i18n::spoken(english);
    kit.query_all_by_role(Role::Button).any(|n| {
        n.accesskit_node().label().as_deref() == Some(&name) && !n.accesskit_node().is_disabled()
    })
}

/// BUG-5, BUG-6: in the «1,234.56» number format (English, Arabic) the suggested amounts are
/// written in that format and taken: the field holds them, the operation's button is on.
#[test]
fn suggested_amounts_are_taken_in_english_and_arabic() {
    use crate::format::{input_text, parse_amount, short_number};
    for lang in ["en", "ar"] {
        for (preset, duffs, action) in [
            ("top-up", 10_000_000, None),
            ("shield-core", 50_000_000, Some("Shield")),
            ("create-identity", 30_000_000, None),
            ("send", 25_000_000, Some("Check and send")),
        ] {
            // What the screen shows and allows follows the demo core's answers, which come on
            // its own thread: waited for, not read at once (the architect's run under load).
            let (_turn, mut kit) = demo_app(preset, lang);
            let what = format!("{preset} {lang}: {} in a field", input_text(duffs));
            wait_for(&mut kit, &what, |kit| {
                amounts(kit).iter().any(|v| parse_amount(v) == Some(duffs))
            });
            if let Some(action) = action {
                let what = format!("{preset} {lang}: «{action}» on");
                wait_for(&mut kit, &what, |kit| enabled(kit, action));
            }
        }
        // The top-up's button says the amount only once it is taken.
        {
            let (_turn, mut kit) = demo_app("top-up", lang);
            let says = |kit: &Kit, duffs| {
                let amount = input_text(duffs);
                kit.query_all_by_role(Role::Button).any(|n| {
                    let a = n.accesskit_node();
                    !a.is_disabled() && a.label().is_some_and(|l| l.contains(&amount))
                })
            };
            wait_for(
                &mut kit,
                &format!("top-up {lang}: «Top up by 0.1»"),
                |kit| says(kit, 10_000_000),
            );
            // The pill «0.5» puts 0.5 in the field, and it is taken too.
            kit.get_by_label(&short_number(50_000_000)).click();
            kit.run_steps(3);
            wait_for(
                &mut kit,
                &format!("top-up {lang}: 0.5 from the pill"),
                |kit| {
                    amounts(kit)
                        .iter()
                        .any(|v| parse_amount(v) == Some(50_000_000))
                },
            );
            wait_for(
                &mut kit,
                &format!("top-up {lang}: «Top up by 0.5»"),
                |kit| says(kit, 50_000_000),
            );
        }
        // `send-confirm`: the last look before signing opens.
        let (_turn, mut kit) = demo_app("send-confirm", lang);
        let name = crate::i18n::spoken("Sign and send");
        wait_for(&mut kit, "«Sign and send»", |kit| {
            kit.query_by_label(&name).is_some()
        });
    }
}

/// BUG-55: from Core the fee is of the coins the amount takes. The demo wallet holds 5, 3 and
/// 0.412 DASH: 0.25 takes one coin (226 duffs), «Max» all three (522) — not one figure for both.
#[test]
fn a_core_payment_costs_the_coins_it_takes() {
    let (_turn, mut kit) = demo_app("send", "en");
    let fee = |kit: &Kit| {
        kit.query_all_by(|n| n.value().is_some_and(|v| v.starts_with("Fee: ")))
            .filter_map(|n| n.accesskit_node().value().map(|v| v.to_string()))
            .next()
            .unwrap_or_default()
    };
    kit.state_mut().send.max = false;
    kit.state_mut().send.amount = "0.25".into();
    kit.run_steps(3);
    assert_eq!(fee(&kit), "Fee: 0.00000226 DASH", "one coin");
    assert!(has_text(&kit, "Network fee: 0.00000226 DASH"));
    let left = 841_200_000 - 25_000_000 - 226;
    assert!(has_text(
        &kit,
        &format!("Left: {}", crate::format::amount(left))
    ));
    // Over the largest coin: two.
    kit.state_mut().send.amount = "6".into();
    kit.run_steps(3);
    assert_eq!(fee(&kit), "Fee: 0.00000374 DASH", "two coins");
    button(&kit, "Max").click();
    kit.run_steps(3);
    assert_eq!(fee(&kit), "Fee: 0.00000522 DASH", "«Max»: every coin");
    assert_eq!(kit.state().send.amount, "8.41199478");
}

/// Sets the open wallet's credits: `shown` of the identity worked as, `all` of every one, for
/// three frames in a row: a snapshot of the demo core in between would put its own back.
fn with_credits(kit: &mut Kit, shown: i64, all: i64) {
    show_wallet_edited(
        kit,
        3,
        "the credits",
        |w| {
            w.balances.credits = shown;
            w.balances.all_credits = all;
            // The credits' sum alone: no mixed coins here (FEAT-0030).
            w.balances.mixed = None;
        },
        |_| true,
    );
}

/// BUG-56 (the customer's decision): «Total» and the «Credits» card count every identity's
/// credits, «2 identities» under it; working as another identity does not change them.
#[test]
fn the_total_counts_every_identitys_credits() {
    let (_turn, mut kit) = demo_app("overview", "en");
    // @satoshi 1.2, @satoshi-lab 0.3.
    with_credits(&mut kit, 120_000_000, 150_000_000);
    let total = crate::format::amount(841_200_000 + 150_000_000 + 73_400_000 + 250_000_000);
    assert!(has_text(&kit, &total), "«Total» with both: {total}");
    assert!(
        has_text(&kit, &crate::format::number(150_000_000, false)),
        "the card (its number, the unit is under it)"
    );
    assert!(has_text(&kit, "DASH · 2 identities"));
    assert!(
        !has_text(&kit, "Credits · @satoshi"),
        "not one identity's card"
    );
    // Working as @satoshi-lab: the same money.
    with_credits(&mut kit, 30_000_000, 150_000_000);
    assert!(has_text(&kit, &total), "«Total» stays");
    assert!(has_text(&kit, &crate::format::number(150_000_000, false)));
}

/// BUG-107: «Accept the request» says what the identity needs for the network to take the write,
/// only while it has less; the price alone led to refusals.
#[test]
fn accept_warns_below_the_balance_the_network_wants() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    wait_for(&mut kit, "a request", |kit| {
        kit.query_all_by_role_and_label(Role::Button, "Accept")
            .next()
            .is_some()
    });
    kit.query_all_by_role_and_label(Role::Button, "Accept")
        .next()
        .expect("«Accept» of the request")
        .click();
    kit.run_steps(4);
    let warning = format!(
        "The network checks a reserve before it writes: with less than ≈ {} on the identity it may refuse. Less is charged.",
        crate::format::amount(230_000)
    );
    with_credits(&mut kit, 126_042, 126_042);
    assert!(has_text(&kit, &warning), "below it: said");
    with_credits(&mut kit, 230_000, 230_000);
    assert!(!has_text(&kit, &warning), "enough: not said");
}

/// BUG-110: «Not enough credits» on Contacts goes once the identity has more; on the screen,
/// not only in the method.
#[test]
fn not_enough_credits_on_contacts_goes_after_a_top_up() {
    let (_turn, mut kit) = demo_app("contacts", "en");
    kit.run_steps(3);
    kit.state_mut().contacts.result = Some(Err(evolite_api::CoreError::NotEnough));
    kit.run_steps(2);
    assert!(kit.state().contacts.result.is_some(), "shown as it was");
    with_credits(&mut kit, 130_000_000, 130_000_000);
    assert!(kit.state().contacts.result.is_none(), "the banner stays");
}

/// BUG-108 r1 must 1: on the screen, from credits the last look says "about"; from Core "at most".
#[test]
fn the_last_look_says_about_for_credits_and_at_most_for_core() {
    // A row reads «label: value» to a screen reader.
    let row = |kit: &Kit, start: &str, mark: &str| {
        kit.query_all_by(|n| {
            n.value()
                .or(n.label())
                .is_some_and(|v| v.starts_with(start) && v.contains(mark))
        })
        .next()
        .is_some()
    };
    let (_turn, mut kit) = demo_app("send-confirm", "en");
    wait_for(&mut kit, "the fee from Core", |kit| {
        row(kit, "Fee, at most: ", "")
    });
    assert!(!row(&kit, "In total: ", "≈"), "from Core, no estimate");
    kit.state_mut().send.pick(evolite_api::Source::Credits);
    kit.run_steps(3);
    assert!(row(&kit, "Fee, about: ", "≈"), "from credits");
    assert!(row(&kit, "In total: ", "≈"), "and the total");
    assert!(!row(&kit, "Fee, at most: ", ""));
}

/// BUG-107 r1 should 1: the Send form's "Add to contacts" is the same write as the dialog's.
#[test]
fn add_to_contacts_on_the_send_form_warns_below_the_balance_the_network_wants() {
    let (_turn, mut kit) = demo_app("send-contact", "en");
    let warning = format!(
        "The network checks a reserve before it writes: with less than ≈ {} on the identity it may refuse. Less is charged.",
        crate::format::amount(230_000)
    );
    // The warning stands under the name once the demo core has found it (an answer of its own,
    // not a snapshot): it is waited for, not counted in frames (BUG-120).
    show_wallet_edited(
        &mut kit,
        3,
        "below it: said",
        |w| {
            w.balances.credits = 126_042;
            w.balances.all_credits = 126_042;
        },
        |k| has_text(k, &warning),
    );
    with_credits(&mut kit, 230_000, 230_000);
    assert!(!has_text(&kit, &warning), "enough: not said");
}

/// BUG-113: from the pool not read yet «Left» is «—», not 0, an amount over it is not called too
/// much, and the button waits for the sync with its reason. «Max»: `max_waits_for_a_balance_to_be_read`.
#[test]
fn the_pool_not_read_yet_leaves_nothing_and_waits() {
    let (_turn, mut kit) = demo_app("send", "en");
    kit.run_steps(3);
    let unread = |kit: &mut Kit| {
        show_wallet_edited(
            kit,
            3,
            "the pool unread",
            |w| {
                w.pool.checked_at = None;
                w.balances.shielded = 0;
            },
            |_| true,
        );
    };
    kit.state_mut().send.pick(evolite_api::Source::Pool);
    kit.state_mut().send.recipient = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f".into();
    kit.state_mut().send.amount = "5".into();
    unread(&mut kit);
    assert!(!has_text(&kit, "More than there is in this balance"));
    assert!(has_words(&kit, "Left: —"), "«Left» is not known");
    assert!(has_words(&kit, crate::text::WAIT_FOR_SYNC));
}

/// BUG-113, the coordinator's check: only the source chosen waits for its balance. From Core,
/// read, a payment goes while the pool is still unread.
#[test]
fn a_payment_from_core_does_not_wait_for_the_pool() {
    let (_turn, mut kit) = demo_app("send", "en");
    kit.run_steps(3);
    kit.state_mut().send.pick(evolite_api::Source::Core);
    kit.state_mut().send.recipient = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f".into();
    kit.state_mut().send.amount = "0.01".into();
    let ready = |kit: &Kit| {
        kit.query_all_by_role_and_label(Role::Button, "Check and send")
            .any(|b| !b.accesskit_node().is_disabled())
    };
    show_wallet_edited(
        &mut kit,
        1,
        "«Check and send» ready",
        |w| {
            w.pool.checked_at = None;
            w.balances.shielded = 0;
        },
        ready,
    );
    let ready = ready(&kit);
    let unread = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .is_some_and(|w| w.pool.checked_at.is_none());
    assert!(unread, "the pool is still not read");
    assert!(ready, "«Check and send» from Core waits for nothing");
    assert!(!has_words(&kit, crate::text::WAIT_FOR_SYNC));
}

/// BUG-113 r1 should 1: the presets show the unread pool for the designer and QA: with an amount
/// kept, the line under the row; with none, no line; «Left» unknown in both.
#[test]
fn the_unread_pool_presets_show_what_they_say() {
    for (preset, line) in [("send-pool-unread", true), ("send-pool-unknown", false)] {
        let (_turn, mut kit) = demo_app(preset, "en");
        kit.run_steps(4);
        assert_eq!(
            has_text(&kit, "The pool: the last known balance, still checking"),
            line,
            "{preset}"
        );
        assert!(has_words(&kit, "Left: —"), "{preset}");
    }
}

/// BUG-115: on Overview, credits kept from an earlier run and not checked in this one say so,
/// without the count of identities; checked, the tile is as before.
#[test]
fn credits_not_checked_yet_say_so_without_the_count() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.run_steps(3);
    let checked = credits_checked_as;
    checked(&mut kit, false);
    assert!(
        has_text(&kit, "DASH · not checked yet"),
        "the credits' line"
    );
    assert!(!has_text(&kit, "DASH · 2 identities"), "no count yet");
    checked(&mut kit, true);
    assert!(has_text(&kit, "DASH · 2 identities"));
    assert!(!has_text(&kit, "DASH · not checked yet"));
}

/// BUG-116: the snapshot's credits, checked in this run or not.
fn credits_checked_as(kit: &mut Kit, checked: bool) {
    show_wallet_edited(
        kit,
        3,
        "the credits checked or not",
        |w| w.balances.credits_checked = checked,
        |_| true,
    );
}

/// BUG-116: «On the identity after» of a top-up is reckoned from the credits: until they are
/// checked in this run it says «—», not a sum built on the kept balance.
#[test]
fn a_top_up_says_what_will_be_on_the_identity_only_once_credits_are_checked() {
    let (_turn, mut kit) = demo_app("identity", "en");
    kit.state_mut().identity.top_up = Some(Default::default());
    credits_checked_as(&mut kit, false);
    assert!(has_text(&kit, "On the identity after: —"), "not checked");
    credits_checked_as(&mut kit, true);
    assert!(!has_text(&kit, "On the identity after: —"));
    assert!(has_words(&kit, "On the identity after: "));
}

/// BUG-116: «Left in credits» of a move into the pool, the same.
#[test]
fn a_move_into_the_pool_says_what_is_left_only_once_credits_are_checked() {
    let (_turn, mut kit) = demo_app("shielded", "en");
    kit.state_mut().pool = crate::screens::shielded::PoolState {
        shielding: true,
        from: 0,
        amount: "0.1".into(),
        ..Default::default()
    };
    credits_checked_as(&mut kit, false);
    assert!(has_text(&kit, "Left in credits: —"), "not checked");
    credits_checked_as(&mut kit, true);
    assert!(!has_text(&kit, "Left in credits: —"));
    assert!(has_words(&kit, "Left in credits: "));
}

/// BUG-116: the snapshot's credits set to `credits`, checked or not.
fn credits_as(kit: &mut Kit, credits: evolite_api::Duffs, checked: bool) {
    show_wallet_edited(
        kit,
        3,
        "the credits",
        |w| {
            w.balances.credits = credits;
            w.balances.credits_checked = checked;
        },
        |_| true,
    );
}

/// BUG-116 (the designer): Add name, Add contact and Profile claim no lack of credits while the
/// credits are not checked in this run; once checked, a real lack is said as before.
#[test]
fn no_lack_of_credits_is_claimed_before_they_are_checked() {
    let lacks = [
        ("add-name", "Not enough credits for the name"),
        (
            "add-contact",
            "The network checks a reserve before it writes",
        ),
    ];
    for (preset, lack) in lacks {
        let (turn, mut kit) = demo_app(preset, "en");
        kit.run_steps(3);
        credits_as(&mut kit, 1, false);
        assert!(!has_words(&kit, lack), "{preset}: not checked yet");
        credits_as(&mut kit, 1, true);
        assert!(has_words(&kit, lack), "{preset}: checked and short");
        drop(kit);
        drop(turn);
    }
    // Profile says nothing of the credits it has, checked or not.
    let (_turn, mut kit) = demo_app("edit-profile", "en");
    kit.run_steps(3);
    credits_as(&mut kit, 1, false);
    assert!(!has_words(&kit, "Not enough credits"));
    assert!(!has_words(&kit, "The network checks a reserve"));
}

/// BUG-112: from the pool to an @name the form says once, under the recipient, that the pool
/// cannot pay a name, promises nothing about privacy, shows no fee, and the button says why.
#[test]
fn from_the_pool_an_at_name_says_so_once_and_promises_nothing() {
    let (_turn, mut kit) = demo_app("send", "en");
    kit.run_steps(3);
    kit.state_mut().send.pick(evolite_api::Source::Pool);
    kit.state_mut().send.recipient = "@alice".into();
    kit.run_steps(4);
    assert!(has_text(
        &kit,
        "You can't send from the pool to an @name. Enter an evolite private address, a Platform address or a Core address, or choose another source."
    ));
    assert!(!has_words(
        &kit,
        "From the pool: the amount and the recipient are not visible"
    ));
    assert!(
        !has_words(&kit, "From the pool you can send to"),
        "the old hint"
    );
    assert!(has_text(&kit, "Change the source or the recipient"));
    assert!(has_words(&kit, "Network fee: —"));
    assert!(has_words(&kit, "Fee: —"));
}

/// The painted text shapes of the frame, nested ones included.
fn painted_galleys(kit: &Kit) -> Vec<std::sync::Arc<egui::Galley>> {
    fn walk(shape: &egui::Shape, out: &mut Vec<std::sync::Arc<egui::Galley>>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.clone()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
}

/// What a line looks like on screen: its letters as drawn, left to right by `x`. None of what
/// does not advance: the dots and marks Noto Sans Arabic draws as glyphs of their own, the
/// zero-width copies epaint adds after a right-to-left run. Spaces are left out.
fn ink_left_to_right(ctx: &egui::Context, text: &str) -> String {
    let galley = ctx.fonts_mut(|f| {
        f.layout_no_wrap(
            text.to_owned(),
            crate::theme::sans(13.0, crate::theme::Weight::Regular),
            egui::Color32::WHITE,
        )
    });
    let mut glyphs: Vec<(char, f32)> = galley
        .rows
        .iter()
        .flat_map(|r| r.row.glyphs.iter().map(move |g| (g, r.pos.x)))
        .filter(|(g, _)| g.advance_width > 0.0 && !g.uv_rect.is_nothing() && !g.chr.is_whitespace())
        .map(|(g, x)| (g.chr, x + g.pos.x))
        .collect();
    glyphs.sort_by(|a, b| a.1.total_cmp(&b.1));
    glyphs.into_iter().map(|g| g.0).collect()
}

/// BUG-117 r2: a contact's line in Arabic that starts with a Latin @name and has a date and
/// numbers in it is drawn as written: in the Arabic interface it reads from the right, «@alice-7»
/// rightmost and left to right, each Arabic word right to left, «24» and «4» whole — every letter
/// once. Cut to a narrow room, it loses whole words at its end (the left) and ends with «…».
/// Contacts cut the line by reading back the laid-out row and turning it to visual order again:
/// the row's text has epaint's copies of each Arabic run, so letters came out doubled and mixed.
#[test]
fn a_contact_line_starting_with_latin_is_drawn_as_written_in_arabic() {
    let (_turn, mut kit) = demo_app("contacts", "ar");
    kit.run_steps(2);
    let ctx = kit.ctx.clone();
    let line = "@alice-7 · اتصال مع 24 أكتوبر · 4 دفعات";
    let font = crate::theme::sans(13.0, crate::theme::Weight::Regular);
    // Left to right on screen: «دفعات» drawn from the right, «4», «·», «أكتوبر», «24», «مع»,
    // «اتصال», «·», «@alice-7».
    let whole = "تاعفد4·ربوتكأ24عملاصتا·@alice-7";
    let wide = crate::screens::contacts::about_shown(&ctx, line, font.clone(), 1000.0);
    assert_eq!(ink_left_to_right(&ctx, &wide), whole, "drawn: {wide:?}");
    let narrow = crate::screens::contacts::about_shown(&ctx, line, font, 150.0);
    let ink = ink_left_to_right(&ctx, &narrow);
    let kept = ink
        .strip_prefix('…')
        .unwrap_or_else(|| panic!("no «…» on the left: {ink:?}"));
    assert!(
        kept.len() < whole.len() && whole.ends_with(kept) && kept.ends_with("@alice-7"),
        "cut: {ink:?}"
    );
}

/// BUG-117 r2, on the screen: every contact's line in Arabic is drawn as its words put in visual
/// order once — Bob's and Alice's lines begin with a Latin name, Kate (without a profile name
/// here) has her @name at the end.
#[test]
fn contact_lines_in_arabic_are_drawn_as_their_words() {
    let (_turn, mut kit) = demo_app("contacts", "ar");
    // What the screen asked for has come: no late snapshot puts Kate's profile name back.
    settle(&mut kit);
    kit.set_size(egui::vec2(1600.0, 1200.0));
    show_wallet_edited(
        &mut kit,
        6,
        "Kate without a profile name",
        |w| {
            for c in &mut w.contacts {
                if c.name.as_deref() == Some("kate") {
                    c.profile.display = None;
                }
            }
        },
        |_| true,
    );
    let contacts = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .expect("a wallet")
        .contacts
        .clone();
    assert!(contacts.len() >= 3, "the demo's contacts");
    let drawn: Vec<String> = painted_galleys(&kit)
        .iter()
        .map(|g| g.text().to_owned())
        .collect();
    for c in &contacts {
        let about = crate::screens::contacts::about_line(c, chrono::Local::now());
        let want = crate::i18n::t(&about).into_owned();
        assert!(
            drawn.contains(&want),
            "{about:?} should be drawn as {want:?}; drawn: {:?}",
            drawn
                .iter()
                .filter(|d| d.chars().any(|ch| ('\u{0600}'..='\u{06FF}').contains(&ch)))
                .collect::<Vec<_>>()
        );
    }
}

/// BUG-119: Contacts' explanation in Arabic takes several rows and is read down from the first:
/// its first words are in the top row and its last word in the bottom one. egui wrapped the
/// line already in visual order, so the top row held the paragraph's end and the bottom its
/// start, with «جهات» alone in a short row.
#[test]
fn contacts_explanation_in_arabic_reads_down_from_its_first_row() {
    const KEY: &str = "DashPay contacts: you exchanged requests. The requests themselves are public; the payment keys inside them are encrypted. Each payment goes to a new address from the contact's keys. A request is a write to Platform paid for by the identity's credits.";
    let (_turn, mut kit) = demo_app("contacts", "ar");
    kit.run_steps(4);
    let said = crate::i18n::t_in(crate::i18n::Lang::Ar, KEY).into_owned();
    let words: Vec<&str> = said.split(' ').collect();
    let (first, last) = (words[0], words[words.len() - 1].trim_end_matches('.'));
    assert!(
        first.chars().any(crate::i18n::is_rtl_char),
        "translated: {said}"
    );
    // The explanation as painted. Its rows are the lines broken before egui saw it — egui broke
    // none of them again — so a row's words are its line's (an Arabic run keeps its letters in
    // reading order in the text; epaint's row counts include the copies it adds, not the text's).
    let galley = painted_galleys(&kit)
        .into_iter()
        .find(|g| {
            let all = g.text();
            all.contains("DashPay") && all.contains(first) && all.contains(last)
        })
        .expect("the explanation");
    let rows: Vec<&str> = galley.text().split('\n').collect();
    assert!(rows.len() > 1, "broken into lines before egui: {rows:?}");
    assert_eq!(galley.rows.len(), rows.len(), "egui broke no line again");
    assert!(
        rows[0].contains(first),
        "«{first}» in the top row: {rows:?}"
    );
    assert!(
        rows[rows.len() - 1].contains(last),
        "«{last}» in the bottom row: {rows:?}"
    );
}

/// BUG-119 (design review): the sync card's line in Arabic, too long for one row in the wide
/// menu, goes down with «· ChainLock» whole: no row ends with the «·» alone.
#[test]
fn the_synced_block_line_keeps_its_dot_with_chainlock() {
    let (_turn, mut kit) = demo_app("contacts", "ar");
    kit.set_size(egui::vec2(1600.0, 1000.0));
    kit.run_steps(6);
    let galley = painted_galleys(&kit)
        .into_iter()
        .find(|g| g.text().contains("ChainLock") && g.text().contains("1,"))
        .expect("the sync card's block line");
    // The «·» shares its row with «ChainLock» (where it stands in the row is the bidi's: in
    // Arabic it is read first, so drawn right of the word).
    for row in galley.text().split('\n') {
        assert!(
            !row.contains('·') || row.contains("ChainLock"),
            "the «·» in a row without «ChainLock»: {:?}",
            galley.text()
        );
    }
}

/// BUG-119 (design review): «Add a contact» in Arabic says under the found profile «… verified
/// @carol …» with the @ on the left of the name, as written.
#[test]
fn the_found_name_on_add_contact_keeps_its_at_on_the_left_in_arabic() {
    let (_turn, mut kit) = demo_app("add-contact", "ar");
    kit.run_steps(6);
    let galley = painted_galleys(&kit)
        .into_iter()
        .find(|g| g.text().contains("carol") && g.text().contains("Platform"))
        .expect("the line under the profile");
    // The glyphs as painted, left to right (not laid out again).
    let mut glyphs: Vec<(char, f32)> = galley
        .rows
        .iter()
        .flat_map(|r| r.row.glyphs.iter().map(move |g| (g, r.pos.x)))
        .filter(|(g, _)| g.advance_width > 0.0 && !g.chr.is_whitespace())
        .map(|(g, x)| (g.chr, x + g.pos.x))
        .collect();
    glyphs.sort_by(|a, b| a.1.total_cmp(&b.1));
    let ink: String = glyphs.into_iter().map(|g| g.0).collect();
    assert!(
        ink.contains("@carol"),
        "drawn {ink:?} from {:?}",
        galley.text()
    );
}

/// BUG-118 (3): Arabic is never drawn in the mono font or letter-spaced: section labels and the
/// pool's line split Arabic words apart that way («اللغة  والتنسيق»), and so did a collection's
/// «رقم 12 من 500» on its tiles and panel (design review). In English the section labels keep
/// their mono with spacing 0.88.
#[test]
fn arabic_is_never_drawn_in_mono_or_letter_spaced() {
    let is_mono = |f: &egui::FontId| {
        f.family == egui::FontFamily::Monospace
            || f.family == egui::FontFamily::Name("mono-500".into())
    };
    for preset in ["settings", "shielded", "activity", "collection"] {
        let (_turn, mut kit) = demo_app(preset, "ar");
        kit.run_steps(4);
        for g in painted_galleys(&kit) {
            for s in &g.job.sections {
                let text = &g.job.text[s.byte_range.start.0..s.byte_range.end.0];
                if !text.chars().any(crate::i18n::is_rtl_char) {
                    continue;
                }
                assert!(
                    !is_mono(&s.format.font_id) && s.format.extra_letter_spacing == 0.0,
                    "{preset}: {text:?} in {:?}, spacing {}",
                    s.format.font_id,
                    s.format.extra_letter_spacing
                );
            }
        }
    }
    let (_turn, mut kit) = demo_app("settings", "en");
    kit.run_steps(4);
    assert!(
        painted_galleys(&kit)
            .iter()
            .any(|g| g
                .job
                .sections
                .iter()
                .any(|s| is_mono(&s.format.font_id) && s.format.extra_letter_spacing == 0.88)),
        "the English section labels keep their mono"
    );
}

/// `f` in the second frame of a context with the app's fonts, in the Arabic interface.
fn in_arabic<T>(mut f: impl FnMut(&mut egui::Ui) -> T) -> T {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, crate::i18n::Lang::Ar, false);
    crate::i18n::set(crate::i18n::Lang::Ar);
    let mut out = None;
    for _ in 0..2 {
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| out = Some(f(ui)));
        frame.textures_delta.clear();
    }
    crate::i18n::set(crate::i18n::Lang::En);
    out.expect("a frame")
}

/// BUG-118 (4): an Arabic note cut to the room in a history row keeps its start and ends with
/// «…»: words go from its end, in reading order. egui's own cut kept the left of the line —
/// the phrase's end in Arabic — and dropped its start.
#[test]
fn a_cut_arabic_note_keeps_its_first_words() {
    const NOTE: &str = "ادفع لصاحب المتجر مقابل الطاولة الخشبية الكبيرة";
    let galley =
        in_arabic(|ui| crate::components::note_galley(ui, NOTE, 140.0, egui::Color32::WHITE));
    let shown = &galley.job.text;
    assert!(
        shown.contains("ادفع") && shown.contains('…') && !shown.contains("الكبيرة"),
        "cut to {shown:?}"
    );
    assert!(galley.size().x <= 140.5, "{}", galley.size().x);
}

/// BUG-118 (1): «Core · 8 نوفمبر» reads from the right: «Core», «·», then the date with its day
/// first, right of its month. Without an isolate the day took the direction of the Latin word
/// before it (bidi rule W7) and stood with «Core»: «نوفمبر … Core · 8».
#[test]
fn a_date_after_a_latin_word_keeps_its_day_with_its_month() {
    let galley = in_arabic(|ui| {
        let line = crate::text::with_date("Core", "8 نوفمبر");
        ui.painter().layout_no_wrap(
            crate::i18n::t(&line).into_owned(),
            crate::theme::sans(14.0, crate::theme::Weight::Medium),
            egui::Color32::WHITE,
        )
    });
    let glyphs: Vec<(char, f32)> = galley.rows[0]
        .glyphs
        .iter()
        .filter(|g| g.advance_width > 0.0 && !g.chr.is_whitespace())
        .map(|g| (g.chr, g.pos.x))
        .collect();
    let x = |c: char| {
        glyphs
            .iter()
            .find(|g| g.0 == c)
            .unwrap_or_else(|| panic!("«{c}» drawn: {glyphs:?}"))
            .1
    };
    let month = glyphs
        .iter()
        .filter(|g| crate::i18n::is_rtl_char(g.0))
        .map(|g| g.1)
        .fold(f32::MIN, f32::max);
    assert!(
        month < x('8') && x('8') < x('C'),
        "drawn {:?}",
        galley.job.text
    );
}

/// BUG-118 (design review): an amount inside an Arabic sentence reads as the card writes it,
/// «0.0003 DASH» left to right; its number and unit had turned around: «DASH 0.0003». The line
/// under a gift's buttons on Collection.
#[test]
fn an_amount_inside_arabic_keeps_its_unit_after_the_number() {
    let galley = in_arabic(|ui| {
        let line = format!(
            "Transfer ≈ {} · a gift cannot be burned",
            crate::format::amount(30_000)
        );
        ui.painter().layout_no_wrap(
            crate::i18n::t(&line).into_owned(),
            crate::theme::sans(12.0, crate::theme::Weight::Regular),
            egui::Color32::WHITE,
        )
    });
    let mut glyphs: Vec<(char, f32)> = galley.rows[0]
        .glyphs
        .iter()
        .filter(|g| g.advance_width > 0.0 && !g.chr.is_whitespace())
        .map(|g| (g.chr, g.pos.x))
        .collect();
    glyphs.sort_by(|a, b| a.1.total_cmp(&b.1));
    let ink: String = glyphs.into_iter().map(|g| g.0).collect();
    assert!(
        ink.contains("0.0003DASH"),
        "drawn {ink:?} from {:?}",
        galley.job.text
    );
}

/// The rect of the first widget named `name`, wholly inside the window: not only in the tree
/// (BUG-133, BUG-127: the tree had them, the window did not).
fn in_the_window(kit: &Kit, name: &str) -> bool {
    let window = kit.ctx.content_rect();
    kit.query_all_by(|n| n.label().as_deref() == Some(name) || n.value().as_deref() == Some(name))
        .next()
        .is_some_and(|n| window.contains_rect(n.rect()))
}

/// BUG-133: «We found your DashPay wallet» shows what it found under its subtitle — the found
/// wallet's rows and «Set the password» are in the window. The eye beside the amounts took the
/// whole height left (a right-to-left layout of its own), and pushed them below the edge.
#[test]
fn the_found_dashpay_wallet_is_in_the_window() {
    let (_turn, mut kit) = demo_app("dashpay", "en");
    let set = crate::i18n::spoken("Set the password");
    wait_for(&mut kit, "the found wallet", |kit| {
        kit.query_by_role_and_label(Role::Button, &set).is_some()
    });
    kit.run_steps(3);
    // Under the found wallet's card (its rows are painted, not in the tree): in the window, the
    // card above it is too.
    assert!(in_the_window(&kit, &set), "«Set the password»");
}

/// BUG-127: «Which wallet to open?» in the smallest window (800×440): the wheel brings «Open»,
/// «Add wallet» and the foot into the window. The screen did not scroll, and they stayed below
/// its edge — the password could only be sent with Enter, and no wallet added.
#[test]
fn the_unlock_screen_scrolls_to_its_buttons_in_the_smallest_window() {
    for lang in ["en", "ar"] {
        let (_turn, mut kit) = demo_app("wallet-unlock", lang);
        kit.set_size(egui::vec2(800.0, 440.0));
        kit.run_steps(4);
        let add = crate::i18n::spoken("Add wallet");
        let open = crate::i18n::spoken("Open");
        assert!(
            !in_the_window(&kit, &add),
            "{lang}: «Add wallet» fits without scrolling: the test shows nothing"
        );
        let centre = kit.ctx.content_rect().center();
        for _ in 0..20 {
            if in_the_window(&kit, &add) {
                break;
            }
            kit.hover_at(centre);
            kit.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            });
            kit.run_steps(3);
        }
        assert!(
            in_the_window(&kit, &add),
            "{lang}: the wheel never brought «Add wallet»"
        );
        assert!(in_the_window(&kit, &open), "{lang}: «Open»");
    }
}

/// BUG-138: Welcome in the smallest window (800×440): the wheel brings «Import from DashPay» and
/// the foot (network, language, theme) into the window. The screen did not scroll, and the third
/// button stood cut at the edge with the foot below it. Since BUG-140 (the compact logo) all of
/// Welcome fits in 800×440: the wheel is checked in a window lower still.
#[test]
fn the_welcome_screen_scrolls_to_its_buttons_and_foot_in_the_smallest_window() {
    for lang in ["en", "ar"] {
        let (_turn, mut kit) = demo_app("welcome", lang);
        kit.set_size(egui::vec2(800.0, 340.0));
        kit.run_steps(4);
        let import = crate::i18n::spoken("Import from DashPay");
        let theme = crate::i18n::spoken("dark theme");
        assert!(
            !in_the_window(&kit, &theme),
            "{lang}: the foot fits without scrolling: the test shows nothing"
        );
        let centre = kit.ctx.content_rect().center();
        for _ in 0..20 {
            if in_the_window(&kit, &theme) {
                break;
            }
            kit.hover_at(centre);
            kit.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            });
            kit.run_steps(3);
        }
        assert!(
            in_the_window(&kit, &theme),
            "{lang}: the wheel never brought the foot"
        );
        assert!(
            in_the_window(&kit, &import),
            "{lang}: «Import from DashPay»"
        );
    }
}

/// BUG-139: in Arabic the unlock screen's links are named for a screen reader in reading
/// order, as `spoken` gives them — not with their words as drawn (visual order), which it read
/// backwards («محفظة إضافة»). With several wallets and with one.
#[test]
fn the_unlock_links_are_named_in_reading_order() {
    for (preset, links) in [
        ("wallet-unlock", &["Forgot the password?", "Add wallet"][..]),
        ("unlock", &["Forgot the password? · Another wallet"][..]),
    ] {
        let (_turn, mut kit) = demo_app(preset, "ar");
        kit.run_steps(4);
        for link in links {
            let spoken = crate::i18n::spoken(link);
            let drawn = crate::i18n::t(link).into_owned();
            assert_ne!(
                spoken, drawn,
                "{link}: the same either way: the test shows nothing"
            );
            assert!(
                kit.query_by_role_and_label(Role::Button, &spoken).is_some(),
                "{preset}: no button named «{spoken}»"
            );
            assert!(
                kit.query_by_role_and_label(Role::Button, &drawn).is_none(),
                "{preset}: «{link}» named in visual order «{drawn}»"
            );
        }
    }
}

/// Pass-22 design review: «Keep in mind» on «Tokens» in Arabic is broken in reading order, as
/// every paragraph since BUG-119: its first words in the top row, its last word in the bottom.
#[test]
fn tokens_keep_in_mind_in_arabic_reads_down_from_its_first_row() {
    const KEY: &str = "Tokens with a zero balance do not appear in the list — add such a token by id. Payouts are not in the balance: if a perpetual distribution may have something to claim, the token is marked “something to claim”. Payouts of distributions to evonodes and of pre-programmed distributions are not shown here. Token balances and transfers are visible to everyone on the network.";
    let (_turn, mut kit) = demo_app("tokens", "ar");
    // Tall enough for the block under the seven rows to be painted.
    kit.set_size(egui::vec2(1024.0, 1400.0));
    kit.run_steps(4);
    let said = crate::i18n::t_in(crate::i18n::Lang::Ar, KEY).into_owned();
    let words: Vec<&str> = said.split(' ').collect();
    let (first, last) = (words[0], words[words.len() - 1].trim_end_matches('.'));
    let galley = painted_galleys(&kit)
        .into_iter()
        .find(|g| g.text().contains(first) && g.text().contains(last))
        .expect("the paragraph");
    let rows: Vec<&str> = galley.text().split('\n').collect();
    assert!(rows.len() > 1, "broken into lines before egui: {rows:?}");
    assert_eq!(galley.rows.len(), rows.len(), "egui broke no line again");
    assert!(
        rows[0].contains(first),
        "«{first}» in the top row: {rows:?}"
    );
    assert!(
        rows[rows.len() - 1].contains(last),
        "«{last}» in the bottom row: {rows:?}"
    );
}
