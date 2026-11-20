//! Settings › Advanced (FEAT-0038 W0, FEAT-0040 «the shared setting»): what most people do not
//! need. «Features for advanced users» is the app's switch, off by default; on, the screens show
//! what FEAT-0040…0046 add ([`crate::context::Ctx::advanced`]); off, those are hidden again at
//! once, and the core refuses what only they would ask. Turning it on or off asks no password and
//! no restart: it is a view, not a spend.

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
}
