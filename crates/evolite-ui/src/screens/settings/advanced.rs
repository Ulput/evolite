//! Settings › Advanced (FEAT-0038 W0, FEAT-0040 «the shared setting»): what most people do not
//! need. «Features for advanced users» is the app's switch, off by default; on, the screens show
//! what FEAT-0040…0046 add ([`crate::context::Ctx::advanced`]); off, those are hidden again at
//! once, and the core refuses what only they would ask. Turning it on or off asks no password and
//! no restart: it is a view, not a spend.
//!
//! «For creators» (FEAT-0038 W0, AC-A1): «Show the Workshop», its own switch, also off by
//! default. Off, it only hides the menu item and the Workshop's screens: what was made stays on
//! the network and in the Collection, and a series going on goes on.

use super::SettingsState;
use super::parts::{Control, group, heading};
use crate::components::Assets;
use crate::theme::Weight;
use egui::Ui;

pub(super) fn advanced(ui: &mut Ui, a: &mut Assets, state: &mut SettingsState) {
    let p = a.p;
    heading(
        ui,
        &p,
        tr!("Advanced"),
        Some(tr!(
            "Features not for every day. Turn them on if you know why."
        )),
    );
    let mut on = state.is_on("advanced");
    group(
        ui,
        a,
        tr!("For advanced users"),
        vec![(
            tr!("Features for advanced users"),
            tr!(
                "Fee choice, a BIP-39 passphrase, the xpub, message signing, a backup file, resending a payment, several recipients. Most people do not need them."
            ),
            Control::Toggle(&mut on),
        )],
    );
    ui.add(crate::components::para(
        tr!(
            "Turning it off only hides these features: money and wallets already made are not touched."
        ),
        12.0,
        Weight::Regular,
        p.muted,
    ));
    *state.toggle("advanced") = on;
    // FEAT-0044 (UC-4): «TOOLS», only with the switch on.
    if on {
        ui.add_space(16.0);
        let pressed = group(
            ui,
            a,
            tr!("Tools"),
            vec![(
                tr!("Sign / check a message"),
                tr!("Prove that a Core address is yours, or check someone's signature"),
                Control::Button(tr!("Open"), crate::components::ButtonKind::Secondary),
            )],
        );
        if pressed == Some(0) {
            state.open_message = true;
        }
    }
    ui.add_space(16.0);
    let mut workshop = state.is_on("workshop");
    group(
        ui,
        a,
        tr!("For creators"),
        vec![(
            tr!("Show the “Workshop”"),
            tr!(
                "Make your own NFTs and tokens. Everything is made on behalf of an identity, costs credits and is visible to everyone on the network."
            ),
            Control::Toggle(&mut workshop),
        )],
    );
    ui.add(crate::components::para(
        tr!(
            "Turning it off only hides the tab: what you made stays on the network and in your “Collection”."
        ),
        12.0,
        Weight::Regular,
        p.muted,
    ));
    *state.toggle("workshop") = workshop;
}
