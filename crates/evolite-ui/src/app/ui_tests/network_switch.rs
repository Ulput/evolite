//! FEAT-0037: testnet and mainnet — what each network shows (the network matrix), the switch
//! on the login screens and in Settings.

use super::*;
use evolite_api::features::{Feature, on};

/// The menu's item named `name` is drawn.
fn in_menu(kit: &Kit, name: &str) -> bool {
    kit.query_all_by(|n| n.label().as_deref() == Some(name))
        .next()
        .is_some()
}

/// The snapshot as on `network`, for the frames until `seen`.
fn on_network(kit: &mut Kit, network: evolite_api::Network, seen: impl Fn(&Kit) -> bool) {
    show_edited(
        kit,
        1,
        "the network",
        |app| {
            if let Some(s) = app.snapshot.as_mut() {
                s.network = network;
            }
        },
        seen,
    );
}

/// AC-T8, AC-A5: each network's menu has what the network matrix lets in; on mainnet now
/// «Tokens» and «Collection» yes (FEAT-0054: the beta on mainnet), «Apps» not yet.
#[test]
fn the_menu_follows_the_network_matrix() {
    use evolite_api::Network::{Mainnet, Testnet};
    for network in [Testnet, Mainnet] {
        let (_turn, mut kit) = demo_app("overview", "en");
        kit.set_size(egui::vec2(1280.0, 800.0));
        on_network(&mut kit, network, |k| in_menu(k, "Overview"));
        for (name, feature) in [
            ("Tokens", Feature::Tokens),
            ("Apps", Feature::Apps),
            ("Collection", Feature::Collection),
        ] {
            assert_eq!(
                in_menu(&kit, name),
                on(feature, network),
                "{network:?}: «{name}»"
            );
        }
        assert!(in_menu(&kit, "Send") && in_menu(&kit, "Private pool"));
    }
}

/// A segment of the network switch, by the name a screen reader hears.
fn segment<'a>(kit: &'a Kit, starts: &str) -> Option<egui_kittest::Node<'a>> {
    let starts = starts.to_owned();
    kit.query_all_by(move |n| n.label().is_some_and(|l| l.starts_with(&starts)))
        .next()
}

/// AC-T6в, AC-A1, AC-A8: the sign-in screen has the network switch, each network named with its
/// wallets for the reader; the list is this network's.
#[test]
fn the_sign_in_screen_has_the_network_switch() {
    let (_turn, mut kit) = demo_app("unlock", "en");
    kit.run_steps(2);
    let n = kit.state().snapshot.as_ref().unwrap().wallets.len();
    assert!(n > 0);
    assert!(
        segment(&kit, &crate::components::network_spoken("Testnet", n)).is_some(),
        "this network and its wallets"
    );
    assert!(
        segment(&kit, "Network: Mainnet, no wallets").is_some(),
        "the other, none there"
    );
}

/// AC-T6б, AC-A1: on the sign-in screen the other network opens at one click, with no dialog:
/// evolite starts again on it, at this window's place, marked as a switch.
#[test]
fn on_sign_in_the_other_network_opens_at_one_click() {
    let (_turn, mut kit) = demo_app("unlock", "en");
    kit.run_steps(2);
    segment(&kit, "Network: Mainnet").expect("Mainnet").click();
    kit.run_steps(3);
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Sign out and go to mainnet")
            .next()
            .is_none(),
        "no dialog"
    );
    let again = kit.state().restarted.clone().expect("started again");
    let again: Vec<String> = again
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let at = |flag: &str| again.iter().position(|a| a == flag);
    assert_eq!(
        at("--network").map(|i| again[i + 1].as_str()),
        Some("mainnet")
    );
    assert!(at("--switched").is_some());
    // The window's place goes too where the window has one (a test harness has none):
    // `network_choice::tests::a_switch_starts_again_on_the_other_network`.
}

/// AC-T6г, AC-A3: in Settings the other network asks first; «Cancel» changes nothing; «Sign out
/// and go to mainnet» locks the wallet, then starts evolite again on mainnet.
#[test]
fn in_settings_the_other_network_asks_first_and_locks() {
    let (_turn, mut kit) = demo_app("settings", "en");
    kit.run_steps(2);
    mainnet_in_settings(&kit).click();
    wait_for(&mut kit, "the dialog", |k| {
        has_text(k, "Switch to mainnet?")
    });
    assert!(has_words(&kit, "Mainnet means real DASH. This is a beta"));
    button(&kit, "Cancel").click();
    kit.run_steps(3);
    assert!(
        kit.query_all_by_role_and_label(Role::Button, "Sign out and go to mainnet")
            .next()
            .is_none(),
        "closed"
    );
    assert!(kit.state().restarted.is_none(), "nothing changed");
    mainnet_in_settings(&kit).click();
    wait_for(&mut kit, "the dialog", |k| {
        has_text(k, "Switch to mainnet?")
    });
    button(&kit, "Sign out and go to mainnet").click();
    // The lock is «Lock»'s own command (the demo has no keys to lock): then evolite starts
    // again on mainnet.
    wait_for(&mut kit, "started again", |k| k.state().restarted.is_some());
    let again = kit.state().restarted.clone().unwrap();
    assert!(again.iter().any(|a| a == "mainnet"), "{again:?}");
}

/// Settings → Network's «Mainnet» (a segment of the network row).
fn mainnet_in_settings(kit: &Kit) -> egui_kittest::Node<'_> {
    kit.query_all_by_role_and_label(Role::RadioButton, "Mainnet")
        .next()
        .expect("Settings → Network: «Mainnet»")
}

const BAND: &str =
    "Mainnet: real DASH. Keep only small amounts here and check the address before every payment.";
const BAND_SHORT: &str = "Mainnet: real DASH. Keep only small amounts here.";

/// AC-A4: on mainnet a band «BETA» over every screen, the sign-in too, its words a status for the
/// reader; the short words in a window narrower than 900; none on testnet.
#[test]
fn mainnet_has_the_beta_band() {
    let band = |kit: &Kit| {
        kit.query_all_by(|n| {
            n.role() == Role::Status && n.label().is_some_and(|l| l.contains("Mainnet: real DASH"))
        })
        .next()
        .and_then(|n| n.accesskit_node().label())
    };
    for preset in ["overview", "unlock"] {
        let (_turn, mut kit) = demo_app(preset, "en");
        kit.set_size(egui::vec2(1024.0, 768.0));
        on_network(&mut kit, evolite_api::Network::Mainnet, |k| {
            band(k).is_some()
        });
        assert_eq!(band(&kit).as_deref(), Some(BAND), "{preset}");
        kit.set_size(egui::vec2(800.0, 600.0));
        on_network(&mut kit, evolite_api::Network::Mainnet, |k| {
            band(k).as_deref() == Some(BAND_SHORT)
        });
        on_network(&mut kit, evolite_api::Network::Testnet, |k| {
            band(k).is_none()
        });
    }
}

/// AC-A2: an empty network beside one with wallets says where they are; with no wallets anywhere
/// (the very first run) nothing is said.
#[test]
fn an_empty_network_says_where_the_wallets_are() {
    const LINE: &str = "There are no mainnet wallets on this computer yet. Your testnet wallets are in the network switch at the top.";
    let (_turn, mut kit) = demo_app("welcome", "en");
    let empty = |app: &mut crate::app::App| {
        if let Some(s) = app.snapshot.as_mut() {
            s.network = evolite_api::Network::Mainnet;
            s.wallets.clear();
        }
    };
    kit.state_mut().networks.other_wallets = 3;
    show_edited(&mut kit, 2, "the line", empty, |k| has_text(k, LINE));
    kit.state_mut().networks.other_wallets = 0;
    show_edited(&mut kit, 2, "no line", empty, |k| !has_text(k, LINE));
}

/// AC-A5, N4: on mainnet Settings → Network explains what is on testnet only, from the network
/// matrix — App Connect, the Platform address's payments; not tokens nor the Collection (on
/// mainnet now, FEAT-0054). Testnet has no such group.
#[test]
fn settings_say_what_is_testnet_only() {
    let (_turn, mut kit) = demo_app("settings", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    on_network(&mut kit, evolite_api::Network::Mainnet, |k| {
        has_text(k, "TESTNET ONLY FOR NOW") || has_text(k, "Testnet only for now")
    });
    for row in [
        "Signing in to sites and apps with App Connect",
        "Payments from a Platform address",
    ] {
        assert!(has_text(&kit, row), "«{row}»");
    }
    assert!(!has_text(
        &kit,
        "NFTs of evolite collections: testnet only for now"
    ));
    assert!(!has_text(
        &kit,
        "Finding, balances and transfers of Dash Platform tokens"
    ));
    on_network(&mut kit, evolite_api::Network::Testnet, |k| {
        !has_text(k, "Payments from a Platform address")
    });
}

/// AC-A5: on mainnet the Platform address's tile, with money on it, says paying from it is on
/// testnet only for now; testnet's does not.
#[test]
fn the_platform_address_tile_says_paying_is_testnet_only() {
    const LINE: &str = "Paying — testnet only for now";
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(egui::vec2(1280.0, 800.0));
    let with_money = |network| {
        move |app: &mut crate::app::App| {
            if let Some(s) = app.snapshot.as_mut() {
                s.network = network;
                if let Some(w) = s.wallet.as_mut() {
                    w.balances.platform_address = 73_400_000;
                    w.balances.platform_address_read = true;
                }
            }
        }
    };
    show_edited(
        &mut kit,
        2,
        "mainnet",
        with_money(evolite_api::Network::Mainnet),
        |k| has_words(k, LINE),
    );
    show_edited(
        &mut kit,
        2,
        "testnet",
        with_money(evolite_api::Network::Testnet),
        |k| !has_words(k, LINE),
    );
}

/// AC-T9, AC-A7: no «test build» on mainnet, nor the rust-dashcore revision, in «About»; none of
/// the catalogs keeps «test build» or «тестовая сборка» at all.
#[test]
fn no_test_build_is_said() {
    let (_turn, mut kit) = demo_app("about", "en");
    kit.set_size(egui::vec2(1280.0, 1000.0));
    on_network(&mut kit, evolite_api::Network::Mainnet, |k| {
        has_text(k, "About")
    });
    assert!(!has_words(&kit, "aca0202"), "the revision");
    assert!(!has_words(&kit, "Test build"));
    for (name, catalog) in [
        ("_keys", include_str!("../../../locales/_keys.json")),
        ("ru", include_str!("../../../locales/ru.json")),
    ] {
        let lower = catalog.to_lowercase();
        assert!(!lower.contains("test build"), "{name}: «test build»");
        assert!(
            !lower.contains("тестовая сборка"),
            "{name}: «тестовая сборка»"
        );
    }
}

/// BUG-263 (the customer): the rail on mainnet has no network mark — the band at the top says
/// it; a red globe said nothing more and looked like an alarm. The sync card sits right above
/// Settings, as on testnet.
#[test]
fn the_rail_on_mainnet_has_no_network_mark() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(egui::vec2(1024.0, 768.0));
    kit.run_steps(2);
    on_network(&mut kit, evolite_api::Network::Mainnet, |k| {
        k.state()
            .snapshot
            .as_ref()
            .is_some_and(|s| s.network == evolite_api::Network::Mainnet)
    });
    kit.run_steps(3);
    assert!(
        kit.query_all_by(|n| {
            n.label().as_deref() == Some("Mainnet · beta")
                || n.value().as_deref() == Some("Mainnet · beta")
        })
        .next()
        .is_none(),
        "no mark on the rail"
    );
    let settings = kit
        .query_all_by(|n| n.label().as_deref() == Some("Settings"))
        .next()
        .expect("Settings")
        .rect();
    let card = kit
        .query_all_by_role(egui::accesskit::Role::Status)
        .next()
        .expect("the sync card")
        .rect();
    assert!(
        card.bottom() <= settings.top(),
        "the card {card:?} above Settings {settings:?}"
    );
}

/// BUG-264 (the customer): the medium screen's menu (the rail, narrower than 1280) has no logo
/// at the top — the sync card's mark is its only one, the switcher high up; the large screen's
/// wide menu keeps its horizontal logo.
#[test]
fn only_the_wide_menu_has_a_logo_at_the_top() {
    let top_logo = |k: &Kit| {
        k.ctx
            .data(|d| d.get_temp::<bool>(egui::Id::new("evolite-menu-top-logo")))
    };
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(egui::vec2(1024.0, 768.0));
    kit.run_steps(3);
    assert_eq!(top_logo(&kit), Some(false), "the rail: none at the top");
    kit.set_size(egui::vec2(1440.0, 900.0));
    kit.run_steps(3);
    assert_eq!(top_logo(&kit), Some(true), "the wide menu keeps it");
}

/// Review r1, Should 1 and P3 2: the run a switch starts ends at its start (a driver, the other
/// network's data): this window stays and says so, and the last network goes back to this one —
/// the next start opens what works. Started, the last network is the new one.
#[test]
fn a_switch_whose_new_run_falls_keeps_this_window() {
    let (_turn, mut kit) = demo_app("unlock", "en");
    let dir = std::env::temp_dir().join(format!("evolite-switch-falls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    kit.state_mut().networks.app_dir = Some(dir.clone());
    kit.run_steps(2);
    // The new run falls before its first frame here.
    kit.state()
        .restart_ends
        .store(true, std::sync::atomic::Ordering::Relaxed);
    segment(&kit, "Network: Mainnet").expect("Mainnet").click();
    wait_for(&mut kit, "the new run", |k| k.state().restarted.is_some());
    wait_for(&mut kit, "the switch's end", |k| {
        k.state().net_switch.is_none()
    });
    kit.run_steps(2);
    assert!(
        has_text(
            &kit,
            "evolite did not start on mainnet. You are still on testnet. Try switching again; if it fails again, close evolite and open it again."
        ),
        "said"
    );
    assert_eq!(
        crate::network_choice::read_last(&dir),
        Some(evolite_api::Network::Testnet),
        "the last network back on this one"
    );
    button(&kit, "Got it").click();
    kit.run_steps(2);
    assert!(
        !has_text(
            &kit,
            "evolite did not start on mainnet. You are still on testnet. Try switching again; if it fails again, close evolite and open it again."
        ),
        "closed"
    );
    // A run that goes on: the last network is the new one.
    kit.state()
        .restart_ends
        .store(false, std::sync::atomic::Ordering::Relaxed);
    segment(&kit, "Network: Mainnet").expect("Mainnet").click();
    kit.run_steps(3);
    assert_eq!(
        crate::network_choice::read_last(&dir),
        Some(evolite_api::Network::Mainnet)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The designer's L2mS: in 800×440 on an empty mainnet with testnet's wallets, the info line and
/// all three buttons are in sight without scrolling, in ru and ar.
#[test]
fn in_the_smallest_window_the_empty_network_shows_all_three_buttons() {
    for lang in ["ru", "ar", "en"] {
        let (_turn, mut kit) = demo_app("welcome", lang);
        kit.set_size(egui::vec2(800.0, 440.0));
        kit.state_mut().networks.other_wallets = 3;
        let empty = |app: &mut crate::app::App| {
            if let Some(s) = app.snapshot.as_mut() {
                s.network = evolite_api::Network::Mainnet;
                s.wallets.clear();
            }
        };
        let import = crate::i18n::spoken("Import from DashPay");
        show_edited(&mut kit, 2, "the screen", empty, |k| {
            k.query_all_by(|n| n.label().as_deref() == Some(import.as_str()))
                .next()
                .is_some()
        });
        let window = kit.ctx.content_rect();
        let rect = kit
            .query_all_by(|n| n.label().as_deref() == Some(import.as_str()))
            .next()
            .expect("the button")
            .rect();
        assert!(
            window.contains_rect(rect),
            "{lang}: «{import}» at {rect:?}, below the edge"
        );
    }
}
