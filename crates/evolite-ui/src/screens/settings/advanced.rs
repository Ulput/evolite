//! Settings › Advanced (FEAT-0038 W0, FEAT-0040 «the shared setting», FEAT-0050 «Mode»): Simple
//! or Advanced, the app's «advanced» switch (Advanced: on), Simple by default. Advanced, the
//! screens show what FEAT-0040…0046 and FEAT-0050's table add ([`crate::context::Ctx::advanced`]);
//! Simple, those are hidden again at once, and the core refuses what only they would ask.
//! Switching asks no password and no restart: it is a view, not a spend.
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
    // FEAT-0050 R-1, UC-2: the mode is the app's «advanced» switch as it was — Advanced is on.
    // At once, no password, no restart: a view, not a spend.
    let mut on = state.is_on("advanced");
    let modes = [tr!("Simple mode"), tr!("Advanced mode")];
    let mut chosen = usize::from(on);
    group(
        ui,
        a,
        tr!("Mode"),
        vec![(
            tr!("What suits you better?"),
            tr!("You can switch at any time. Your money and data stay as they are."),
            Control::Segmented(&modes, &mut chosen),
        )],
    );
    on = chosen == 1;
    *state.toggle("advanced") = on;
    // FEAT-0044 (UC-4): «TOOLS», only with the switch on.
    if on {
        ui.add_space(16.0);
        let pressed = group(
            ui,
            a,
            tr!("Tools"),
            vec![
                (
                    tr!("Sign / check a message"),
                    tr!("Prove that a Core address is yours, or check someone's signature"),
                    Control::Button(tr!("Open"), crate::components::ButtonKind::Secondary),
                ),
                // FEAT-0054: the fallback for a collection the search did not find.
                (
                    tr!("Add a collection by id"),
                    tr!("Show your NFTs of an evolite collection the Collection did not find"),
                    Control::Button(tr!("Open"), crate::components::ButtonKind::Secondary),
                ),
            ],
        );
        match pressed {
            Some(0) => state.open_message = true,
            Some(1) => state.collections.open = true,
            _ => {}
        }
    }
    // FEAT-0050: the Workshop and its switch only in Advanced; in Simple it counts as off, its
    // own switch kept for when the mode is Advanced again.
    if !on {
        return;
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
