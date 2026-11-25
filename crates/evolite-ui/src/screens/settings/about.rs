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
                "Windows notifications while evolite is open but not in front. Amounts are hidden if “Hide amounts” is on."
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
                // FEAT-0051 R-7: the Dash terms only for advanced users.
                if crate::text::advanced() {
                    tr!("InstantLock and ChainLock for your payments")
                } else {
                    ""
                },
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

/// Р1 (design REL-verify, review r1): a newer release with its links, when the last check was
/// believed, and the release keys with their ids, so the person can compare them with
/// docs/verify.md and README.md.
fn updates(ui: &mut Ui, a: &mut Assets, update: Option<&evolite_api::UpdateInfo>) {
    let mut asked = None;
    let mut rows = Vec::new();
    let newer = update.filter(|u| u.newer);
    let new_line = newer.map(|u| format!(tr!("evolite {} is out"), u.version));
    let released = newer.map(|u| {
        format!(
            tr!("Released {}. Check the installer before you install it."),
            u.date
        )
    });
    if let (Some(line), Some(released)) = (&new_line, &released) {
        rows.push((
            line.as_str(),
            released.as_str(),
            Control::Buttons(
                &[
                    (tr!("Release page"), ButtonKind::Primary),
                    (tr!("How to check the installer"), ButtonKind::Secondary),
                ],
                &mut asked,
            ),
        ));
    }
    let checked = match update {
        Some(u) => format!(
            tr!("Checked {}: a signed answer"),
            crate::text::date_long(u.checked_at)
        ),
        None => tr!("Not checked yet, or the check is off").to_owned(),
    };
    rows.push((
        tr!("Last check"),
        checked.as_str(),
        Control::Text("", false),
    ));
    let keys: Vec<String> = evolite_api::RELEASE_KEYS
        .iter()
        .map(|key| {
            let id = evolite_api::release_key_id(key).unwrap_or_default();
            format!("{key} · {id}")
        })
        .collect();
    let keys = if keys.is_empty() {
        tr!("Published with the first signed release").to_owned()
    } else {
        keys.join("\n")
    };
    rows.push((
        tr!("Release keys"),
        tr!("minisign public keys and their key IDs: compare them with docs/verify.md"),
        Control::Text(&keys, true),
    ));
    group(ui, a, tr!("Updates"), rows);
    if let (Some(which), Some(u)) = (asked, newer) {
        crate::links::open(if which == 0 {
            &u.notes_url
        } else {
            &u.verify_url
        });
    }
}

pub(super) fn about(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut SettingsState,
    update: Option<&evolite_api::UpdateInfo>,
) {
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
                    // No «test build» any more (FEAT-0037 AC-A7): mainnet is a beta (М9).
                    evolite_api::Network::Testnet => tr!("Testnet: the coins are worth nothing"),
                    evolite_api::Network::Mainnet => {
                        tr!("Mainnet beta: real DASH, keep only small amounts")
                    }
                },
                Control::Text(env!("CARGO_PKG_VERSION"), true),
            ),
            (
                "Dash Core",
                tr!(
                    "dash-spv: headers, block filters (BIP158), ChainLock and InstantSend are verified here"
                ),
                // No revision: it moves with the dependencies (FEAT-0037 AC-T9).
                Control::Text("rust-dashcore", true),
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
    updates(ui, a, update);
    group(
        ui,
        a,
        tr!("What it does"),
        vec![
            (
                tr!("Implemented"),
                tr!(
                    "Core: receive, send, history, your own node. Platform: identities, DPNS names, credits, Platform address. DashPay contacts and profile, the private pool, app sign-in by App Connect v2 with a spending limit and a period, NFT collections of the evolite standard, testnet and mainnet (beta), the network is chosen on the sign-in screen"
                ),
                Control::Text("", false),
            ),
            (
                tr!("Not yet"),
                tr!("Giving, selling and receiving NFTs by link"),
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
