//! FEAT-0021 on the demo core: a contested name's check (N1), its review (N2) with the fund «not
//! returned» before the password (AC-T6), the request sent (N3), «DPNS names» in every state of a
//! contest (N4, AC-T8), and the contest card (N5) opened from its row (AC-T21, entry 1).

use super::*;

/// Whether the button called `name` is drawn, and on.
fn on(kit: &Kit, name: &str) -> Option<bool> {
    kit.query_all_by_role(Role::Button)
        .find(|b| b.accesskit_node().label().as_deref() == Some(name))
        .map(|b| !b.accesskit_node().is_disabled())
}

/// N1: a contested name is answered with the vote, its fund and span, and the similar letters;
/// «Check the request» opens the review (N2).
#[test]
fn a_contested_name_is_answered_with_its_vote_and_goes_to_the_review() {
    let (_turn, mut kit) = demo_app("add-name-contested", "en");
    wait_for(&mut kit, "the contested answer", |k| {
        has_words(
            k,
            "@sam is a contested name: masternodes award it by vote. Fund ",
        )
    });
    assert!(has_words(&kit, "Similar letters are one name"), "R-6");
    wait_for(&mut kit, "«Check the request» on", |k| {
        on(k, "Check the request") == Some(true)
    });
    button(&kit, "Check the request").click();
    wait_for(&mut kit, "the review", |k| {
        has_text(k, "Check the name request")
    });
    assert!(has_words(&kit, "Fund for the vote"));
    assert!(has_words(&kit, "The masternode vote lasts "));
}

/// AC-T6 and AC-D2: the review says the fund is not returned, above the password, in a wide
/// window and in the smallest (800 × 440, the footer kept in the window, BUG-122); «Sign and send»
/// waits for the password.
#[test]
fn the_review_says_the_fund_is_not_returned_above_the_password() {
    for size in [egui::vec2(1024.0, 768.0), egui::vec2(800.0, 440.0)] {
        let (_turn, mut kit) = demo_app("add-name-review", "en");
        kit.set_size(size);
        kit.run_steps(4);
        wait_for(&mut kit, "the review", |k| {
            has_text(k, "Check the name request")
        });
        let warning = kit
            .query_all_by(|n| n.value().is_some_and(|v| v.contains("is not returned")))
            .next()
            .unwrap_or_else(|| panic!("{size:?}: the fund «not returned»"))
            .rect();
        let password = kit
            .query_all_by_role(Role::PasswordInput)
            .next()
            .unwrap_or_else(|| panic!("{size:?}: the password"))
            .rect();
        assert!(
            warning.bottom() <= password.top(),
            "{size:?}: above the password: {warning:?} / {password:?}"
        );
        assert!(
            warning.bottom() <= size.y && warning.top() >= 0.0,
            "{size:?}: in the window: {warning:?}"
        );
        assert_eq!(
            on(&kit, "Sign and send the request"),
            Some(false),
            "{size:?}: not before the password"
        );
    }
}

/// R-2: a fund risen since the review is shown, was and now, before it is signed again.
#[test]
fn a_fund_risen_since_the_review_is_shown_before_signing_again() {
    let (_turn, mut kit) = demo_app("add-name-fund-rose", "en");
    wait_for(&mut kit, "the risen fund", |k| {
        has_words(k, "The fund rose: it was ")
    });
}

/// N3: the request is in, it is not the identity's name yet, and the card is a click away.
#[test]
fn a_sent_request_says_it_is_in_the_vote() {
    let (_turn, mut kit) = demo_app("add-name-sent", "en");
    wait_for(&mut kit, "the request sent", |k| {
        has_text(k, "The request for @sam is in the vote")
    });
    assert!(has_words(
        &kit,
        "While the vote lasts the name is not yours yet"
    ));
    assert_eq!(on(&kit, "Open the request"), Some(true));
}

/// AC-T8 and AC-D5: «DPNS names» shows every state of a contest — in the vote with its claims and
/// votes, waiting for the result, won, lost and blocked with the fund not returned and when they
/// leave the list — with «Claim another ›» and «Hide» on the lost and the blocked one.
#[test]
fn dpns_names_show_every_state_of_a_contest() {
    let (_turn, mut kit) = demo_app("names-contested", "en");
    wait_for(&mut kit, "the names", |k| has_text(k, "@nakamoto-dev"));
    for line in [
        "masternode vote · ",
        "the vote is over · waiting for the result",
        "went to another identity · the fund ",
        "blocked by the masternodes · the fund ",
        "leaves the list in ",
    ] {
        assert!(has_words(&kit, line), "«{line}»");
    }
    assert_eq!(
        kit.query_all_by_role(Role::Button)
            .filter(|b| b.accesskit_node().label().as_deref() == Some("Hide"))
            .count(),
        2,
        "«Hide» on the lost and the blocked name"
    );
    // A hidden one leaves the list at once (R-12): what the core says is no longer listed.
    show_wallet_edited(
        &mut kit,
        1,
        "the hidden name gone",
        |w| {
            if let Some(id) = w.identity.as_mut() {
                for n in &mut id.names {
                    if n.label == "bob"
                        && let Some(c) = n.contest.as_mut()
                    {
                        c.listed = false;
                    }
                }
            }
        },
        |k| !has_text(k, "@bob") && has_text(k, "@zed"),
    );
}

/// AC-T21, entry 1: a claimed name's row opens its contest card — the claims with their votes,
/// this identity's marked, the votes to block, and the rule of the result.
#[test]
fn a_claimed_names_row_opens_its_contest_card() {
    let (_turn, mut kit) = demo_app("names-contested", "en");
    wait_for(&mut kit, "the names", |k| has_text(k, "@nakamoto-dev"));
    click_words_of(&mut kit, "@nakamoto-dev");
    wait_for(&mut kit, "the card", |k| {
        has_text(k, "@nakamoto-dev — vote")
    });
    assert!(
        has_words(&kit, "· your identity"),
        "this identity's claim marked"
    );
    assert!(has_words(&kit, "Against all (block)"));
    assert!(has_words(&kit, "The most votes win."));
    assert!(has_words(&kit, "Votes proved by Platform · updated "));
}

/// N5b: the lost name's card has its result and «Hide from the list», which closes it.
#[test]
fn a_lost_names_card_offers_to_hide_it() {
    let (_turn, mut kit) = demo_app("contest-lost", "en");
    wait_for(&mut kit, "the card", |k| {
        has_text(k, "@bob — result of the vote")
    });
    assert!(has_words(&kit, "· got the name"), "the winner marked");
    assert!(has_words(&kit, "leaves the list in "));
    button(&kit, "Hide from the list").click();
    wait_for(&mut kit, "the card closed", |k| {
        !has_text(k, "@bob — result of the vote")
    });
}

/// AC-T22, entry 4: a click on a vote's result note opens Identity with the claim's contest card.
#[cfg(windows)]
#[test]
fn a_click_on_the_results_note_opens_the_contest_card() {
    let (_turn, mut kit) = demo_app("names-contested", "en");
    wait_for(&mut kit, "the names", |k| has_text(k, "@nakamoto-dev"));
    let id = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| w.identity.as_ref())
        .map(|i| i.id.clone())
        .expect("the identity");
    kit.state_mut().screen = crate::screens::Screen::Overview;
    kit.state_mut().notifier.clicked(&id, "bob");
    wait_for(&mut kit, "the card", |k| {
        has_text(k, "@bob — result of the vote")
    });
    assert_eq!(kit.state().screen, crate::screens::Screen::Identity);
}

/// AC-T19 and AC-T21, entry 3: a hidden lost name leaves «DPNS names», History keeps the claim's
/// line (the fund paid) and the result's; the result's card opens the contest card with its
/// result, without «Hide from the list».
#[test]
fn history_keeps_a_hidden_names_lines_and_opens_its_card() {
    let (_turn, mut kit) = demo_app("contest-lost-hidden", "en");
    wait_for(&mut kit, "the names", |k| has_text(k, "@nakamoto-dev"));
    assert!(!has_text(&kit, "@bob"), "hidden: not listed");
    kit.state_mut().screen = crate::screens::Screen::Activity;
    wait_for(&mut kit, "History's lines", |k| {
        has_words(k, "The name @bob went to another identity")
            && has_words(k, "Request for the name @bob")
    });
    crate::screens::open_card(&kit.ctx, 9_021_000);
    wait_for(&mut kit, "the result's card", |k| {
        has_text(k, "Open the vote ›")
    });
    button(&kit, "Open the vote ›").click();
    wait_for(&mut kit, "the contest card", |k| {
        has_text(k, "@bob — result of the vote")
    });
    assert_eq!(kit.state().screen, crate::screens::Screen::Identity);
    assert!(!has_text(&kit, "Hide from the list"), "already hidden");
}

/// BUG-191: a claim's card names the fund and the writes apart — «Fund for the vote · not
/// refunded» and «Writes to Platform» — and calls nothing a fee.
#[test]
fn a_claims_card_names_the_fund_and_the_writes() {
    let (_turn, mut kit) = demo_app("contest-lost-hidden", "en");
    kit.state_mut().screen = crate::screens::Screen::Activity;
    wait_for(&mut kit, "the claim's line", |k| {
        has_words(k, "Request for the name @bob")
    });
    crate::screens::open_card(&kit.ctx, 9_021_002);
    wait_for(&mut kit, "the claim's card", |k| {
        has_words(k, "Fund for the vote · not refunded: ")
    });
    assert!(has_words(&kit, "Writes to Platform: "));
    assert!(!has_words(&kit, "Fee: "), "the fund is no fee");
}

/// AC-T21, entry 2: Overview has the notice of a name in its vote, and «Open the vote» opens its
/// card on Identity; «×» takes the notice away.
#[test]
fn overview_shows_a_name_in_its_vote_and_opens_its_card() {
    let (_turn, mut kit) = demo_app("overview", "en");
    wait_for(&mut kit, "the notice", |k| {
        has_words(k, "The name @nakamoto-dev is in the masternode vote · ")
    });
    button(&kit, "Open the vote").click();
    wait_for(&mut kit, "the card", |k| {
        has_text(k, "@nakamoto-dev — vote")
    });
    assert_eq!(kit.state().screen, crate::screens::Screen::Identity);
}

/// AC-T21, entry 5: «Add a name» on a name this identity already asked for says so and opens its
/// card; after a request is sent, «Open the request» asks for that request's card.
#[test]
fn add_a_name_leads_to_the_request_it_already_has() {
    let (_turn, mut kit) = demo_app("add-name", "en");
    kit.state_mut()
        .identity
        .adding_name
        .as_mut()
        .expect("the form")
        .name = "nakamoto-dev".into();
    wait_for(&mut kit, "the answer", |k| {
        has_words(
            k,
            "You are already in the vote for @nakamoto-dev from this identity",
        )
    });
    button(&kit, "Open the request ›").click();
    wait_for(&mut kit, "the card", |k| {
        has_text(k, "@nakamoto-dev — vote")
    });

    drop((_turn, kit));
    let (_turn, mut kit) = demo_app("add-name-sent", "en");
    wait_for(&mut kit, "the request sent", |k| {
        has_text(k, "Open the request")
    });
    button(&kit, "Open the request").click();
    kit.run_steps(2);
    assert!(
        kit.state().identity.adding_name.is_none(),
        "the dialog gives way"
    );
}

/// Review r1, Should 1: «×» on Overview's notice of the vote takes it away and the core keeps it
/// closed (the demo as the file does); the vote's other entries stay.
#[test]
fn overviews_notice_closed_stays_closed() {
    let (_turn, mut kit) = demo_app("overview", "en");
    wait_for(&mut kit, "the notice", |k| {
        has_words(k, "The name @nakamoto-dev is in the masternode vote · ")
    });
    let cross = kit
        .query_all_by_role(Role::Button)
        .find(|b| {
            b.accesskit_node()
                .label()
                .is_some_and(|l| l.contains("Close"))
        })
        .expect("the notice's «×»");
    cross.click();
    wait_for(&mut kit, "the notice gone", |k| {
        !has_words(k, "The name @nakamoto-dev is in the masternode vote · ")
    });
    settle(&mut kit);
    assert!(
        !has_words(&kit, "The name @nakamoto-dev is in the masternode vote · "),
        "the next snapshot keeps it closed"
    );
}

/// AC-T23 (UC-5): «Create an identity» with a contested name makes the identity without it, says
/// the claim and its fund before the password, and Identity then offers the claim, whose review
/// (N2) opens at once — two spends, two passwords.
#[test]
fn a_contested_name_at_the_creation_is_claimed_right_after() {
    let (_turn, mut kit) = demo_app("create-identity-nameless", "en");
    kit.set_size(egui::vec2(1024.0, 1000.0));
    kit.state_mut().create_identity.name = "sam".into();
    wait_for(&mut kit, "the contested answer", |k| {
        has_words(
            k,
            "@sam is a contested name: masternodes award it by vote. The identity is made now without it",
        )
    });
    wait_for(&mut kit, "«Create for …» on", |k| {
        k.query_all_by_role(Role::Button).any(|b| {
            b.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Create for "))
                && !b.accesskit_node().is_disabled()
        })
    });
    kit.query_all_by_role(Role::Button)
        .find(|b| {
            b.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Create for "))
        })
        .expect("«Create for …»")
        .click();
    kit.run_steps(3);
    assert!(has_text(&kit, "Check the new identity"));
    assert!(
        has_words(
            &kit,
            "@sam is not part of this: masternodes award it by vote."
        ),
        "the claim and its fund, before the password"
    );
    button(&kit, "Sign and create").click();
    wait_for(&mut kit, "Identity offering the claim", |k| {
        k.state().screen == crate::screens::Screen::Identity && has_text(k, "Claim @sam ›")
    });
    let named = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .and_then(|w| w.identity.as_ref())
        .is_some_and(|i| i.names.iter().any(|n| n.label == "sam"));
    assert!(!named, "made without the contested name");
    button(&kit, "Check the request").click();
    wait_for(&mut kit, "the review", |k| {
        has_text(k, "Check the name request")
    });
}

/// АРХ-6, AC-T11 and AC-T14: the review takes a link «where my identity is confirmed» only
/// https within 128 characters, said under the field; a good one has a line of its own, «≈».
#[test]
fn the_reviews_link_is_checked_and_has_a_line_of_its_own() {
    let (_turn, mut kit) = demo_app("add-name-review", "en");
    kit.set_size(egui::vec2(1024.0, 1000.0));
    wait_for(&mut kit, "the review", |k| {
        has_text(k, "Check the name request")
    });
    assert!(has_words(
        &kit,
        "Link where your identity is confirmed · optional"
    ));
    let set = |kit: &mut Kit, link: &str| {
        kit.state_mut()
            .identity
            .adding_name
            .as_mut()
            .expect("the form")
            .link = link.into();
        kit.run_steps(3);
    };
    set(&mut kit, "http://example.com/me");
    assert!(has_words(
        &kit,
        "A link https:// of at most 128 characters is needed"
    ));
    assert!(!has_words(&kit, "Identity confirmation"));
    set(&mut kit, "https://example.com/me");
    assert!(has_words(
        &kit,
        "Everyone sees the link; voters in DashPay see it on the request's card."
    ));
    assert!(has_words(&kit, "Identity confirmation: ≈ "), "its own line");
}

/// АРХ-6, AC-T12: a link the claim kept but Platform did not take is said on the contest card, and
/// written again alone, with a password: never the fund again.
#[test]
fn a_link_not_written_is_offered_again_alone() {
    let (_turn, mut kit) = demo_app("contest-card", "en");
    wait_for(&mut kit, "the card", |k| {
        has_text(k, "@nakamoto-dev — vote")
    });
    assert!(has_words(
        &kit,
        "Identity confirmation: https://example.com/sam"
    ));
    assert!(has_words(
        &kit,
        "The request is in; the link was not written."
    ));
    assert!(
        kit.query_all_by_role(Role::Button)
            .any(|b| b.accesskit_node().label().as_deref() == Some("Write the link again")),
        "the retry"
    );
}
