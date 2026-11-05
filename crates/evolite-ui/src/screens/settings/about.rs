//! Notifications and About.

use super::SettingsState;
use super::parts::{Control, group, heading};
use crate::components::{Assets, ButtonKind};
use egui::Ui;

pub(super) fn notifications(ui: &mut Ui, a: &mut Assets, state: &mut SettingsState) {
    let p = a.p;
    heading(
        ui,
        &p,
        tr!("Notifications"),
        Some(if crate::notify::SUPPORTED {
            tr!(
                "Windows notifications while evolite is open but not in front. Amounts are hidden if «Hide amounts» is on."
            )
        } else {
            tr!(
                "System notifications are not available on this system yet: evolite supports them only on Windows."
            )
        }),
    );
    let mut incoming = *state.toggle("notify-incoming");
    let mut locks = *state.toggle("notify-locks");
    let mut apps = *state.toggle("notify-apps");
    group(
        ui,
        a,
        tr!("Events"),
        vec![
            (
                tr!("Incoming payments"),
                tr!("As soon as a payment appears on the network"),
                Control::Toggle(&mut incoming),
            ),
            (
                tr!("Confirmations"),
                tr!("InstantLock and ChainLock for your payments"),
                Control::Toggle(&mut locks),
            ),
            (
                tr!("App requests"),
                tr!("Sign-in and signing by link or QR — once apps arrive in the real wallet"),
                Control::Toggle(&mut apps),
            ),
        ],
    );
    *state.toggle("notify-incoming") = incoming;
    *state.toggle("notify-locks") = locks;
    *state.toggle("notify-apps") = apps;
}

/// Where «Source code · GitHub» leads (customer, 2026-11-05).
pub(super) const SOURCE_CODE: &str = "https://github.com/Ulput/evolite";

pub(super) fn about(ui: &mut Ui, a: &mut Assets, state: &mut SettingsState) {
    let p = a.p;
    heading(
        ui,
        &p,
        tr!("About"),
        Some(tr!(
            "A light Dash wallet: everything the network sends is verified on this computer."
        )),
    );
    group(
        ui,
        a,
        "evolite",
        vec![
            (
                tr!("Version"),
                match evolite_core::network::current() {
                    evolite_api::Network::Testnet => {
                        tr!("Test build: testnet, the coins are worth nothing")
                    }
                    evolite_api::Network::Mainnet => {
                        tr!("Test build on mainnet: real DASH, small amounts only")
                    }
                },
                Control::Text(env!("CARGO_PKG_VERSION"), true),
            ),
            (
                "Dash Core",
                tr!(
                    "dash-spv: headers, block filters (BIP158), ChainLock and InstantSend are verified here"
                ),
                Control::Text("rust-dashcore aca0202", true),
            ),
            (
                "Dash Platform",
                tr!(
                    "platform-wallet and dash-sdk: answers come with GroveDB proofs, checked with quorum keys from our own masternode list"
                ),
                Control::Text("platform 575d961", true),
            ),
            (
                tr!("Storage"),
                tr!(
                    "The phrase is encrypted with the password (Argon2, AES-GCM); history, identities and contacts are in SQLite on this computer"
                ),
                Control::Text(tr!("locally"), false),
            ),
            (
                tr!("Pictures"),
                tr!("NFTs and avatars are drawn by a separate process with no access to keys"),
                Control::Text(tr!("in a sandbox"), false),
            ),
        ],
    );
    group(
        ui,
        a,
        tr!("What it does"),
        vec![
            (
                tr!("Implemented"),
                tr!(
                    "Core: receive, send, history, your own node. Platform: identities, DPNS names, credits, Platform address. DashPay contacts and profile, the private pool, app sign-in and signing by link, the Jembe NFT collection, mainnet by a launch flag"
                ),
                Control::Text("", false),
            ),
            (
                tr!("Not yet"),
                tr!("Giving, selling and receiving NFTs by link; sign-in by code (dash-kx:)"),
                Control::Text("", false),
            ),
        ],
    );
    let clicked = group(
        ui,
        a,
        tr!("Contact"),
        vec![
            (
                tr!("Source code"),
                tr!("Open, MIT"),
                Control::Value("GitHub", false),
            ),
            (
                tr!("Report a problem"),
                tr!(
                    "An email to the developers from your own mail: you will see all of it before sending"
                ),
                Control::Button(tr!("Write"), ButtonKind::Secondary),
            ),
        ],
    );
    match clicked {
        // The browser, as the explorer link opens it (BUG-28).
        Some(0) => {
            crate::links::open(SOURCE_CODE);
        }
        Some(1) => state.report = Some(Default::default()),
        _ => {}
    }
    let note = state.journal_note.clone().unwrap_or_else(|| {
        tr!("What the wallet did: sync stages, commands and their outcome, errors. No phrase, keys, addresses, amounts or names; kept for 7 days").to_string()
    });
    // One row, so both buttons are in sight together.
    let mut clicked = None;
    group(
        ui,
        a,
        tr!("Journal"),
        vec![(
            tr!("Journal for reports"),
            &note,
            Control::Buttons(
                &[
                    (tr!("Open folder"), ButtonKind::Secondary),
                    (tr!("Clear journal"), ButtonKind::Ghost),
                ],
                &mut clicked,
            ),
        )],
    );
    match clicked {
        Some(0) => {
            if let Some(dir) = evolite_core::diag::dir() {
                super::report::show(&dir);
            }
        }
        Some(1) => {
            state.journal_note = Some(match evolite_core::diag::clear() {
                Ok(()) => tr!("Journal cleared: new records start from here").to_string(),
                Err(e) => format!(
                    tr!("The journal was not cleared: {}"),
                    evolite_core::diag::io_error_kind(&e)
                ),
            });
        }
        _ => {}
    }
}
