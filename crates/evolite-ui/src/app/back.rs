//! BUG-169: a screen entered from several places goes back to where it was entered from —
//! «Cancel», the «‹ {place}» link above its title, Esc without a dialog and the mouse's «back»
//! (АРХ-1). One step deep; the place's own state (scroll, filters, an open profile or card)
//! lives in `App` across screens already, so the screen alone is remembered.

use crate::screens::Screen;

/// Where a returning screen goes back to, and what it was entered with: another wallet or
/// identity since then and there is nothing to go back to (R-3).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Back {
    pub screen: Screen,
    wallet: Option<String>,
    identity: Option<String>,
}

/// The open wallet and the identity worked as: what a place to go back to belongs to.
pub(crate) type Here = (Option<String>, Option<String>);

/// The open wallet and the identity worked as in `snapshot`.
pub(crate) fn here(snapshot: &evolite_api::Snapshot) -> Here {
    (
        snapshot
            .wallets
            .iter()
            .find(|w| w.open)
            .map(|w| w.id.clone()),
        snapshot
            .wallet
            .as_ref()
            .and_then(|w| w.identity.as_ref())
            .map(|i| i.id.clone()),
    )
}

/// Esc or the mouse's «back» this frame, on the screen itself (R-6): not when a dialog or a
/// menu is open — Esc there closes only it (a dialog takes its Esc as it closes).
pub(crate) fn esc_or_mouse_back(ctx: &egui::Context) -> bool {
    if ctx.memory(|m| m.top_modal_layer().is_some()) || ctx.any_popup_open() {
        return false;
    }
    ctx.input(|i| {
        i.key_pressed(egui::Key::Escape) || i.pointer.button_pressed(egui::PointerButton::Extra1)
    })
}

/// The screens that go back to where they were entered from. FEAT-0020 adds the keys' screen
/// (K1).
pub(crate) fn returning(screen: Screen) -> bool {
    matches!(screen, Screen::Send)
}

/// The place to go back to after a move from `from` to `to` (AC-T1): set on entering a
/// returning screen from another; kept while on one, also from one returning screen to another
/// (one step deep, R-4); none on any other screen.
pub(crate) fn back_after(from: Screen, to: Screen, back: Option<Back>, here: Here) -> Option<Back> {
    if !returning(to) {
        return None;
    }
    if from == to || returning(from) {
        return back;
    }
    let (wallet, identity) = here;
    Some(Back {
        screen: from,
        wallet,
        identity,
    })
}

/// Where going back leads (AC-T2): the place remembered while it is still there — the same
/// wallet and identity, a screen with the menu that this network has (FEAT-0037) — else
/// Overview (R-3).
pub(crate) fn back_to(back: Option<&Back>, here: &Here, network: evolite_api::Network) -> Screen {
    match back {
        Some(b)
            if (&b.wallet, &b.identity) == (&here.0, &here.1)
                && has_menu(b.screen)
                && crate::screens::shown_on(b.screen, network) =>
        {
            b.screen
        }
        _ => Screen::Overview,
    }
}

/// A screen of the open wallet, with the menu: not the welcome, the unlock or the onboarding.
fn has_menu(screen: Screen) -> bool {
    !matches!(
        screen,
        Screen::Welcome | Screen::Unlock | Screen::Onboarding(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::Network::{Mainnet, Testnet};

    fn here(wallet: &str, identity: Option<&str>) -> Here {
        (Some(wallet.into()), identity.map(Into::into))
    }

    /// AC-T1: set on entering Send from another screen (from the menu too, R-2); kept while on
    /// Send; none once Send is left.
    #[test]
    fn the_place_is_set_on_entering_and_kept_inside() {
        let w = here("w1", Some("id1"));
        let set = back_after(Screen::Contacts, Screen::Send, None, w.clone());
        assert_eq!(set.as_ref().map(|b| b.screen), Some(Screen::Contacts));
        // Inside Send (another source, «Check the payment», coins): the same screen.
        let kept = back_after(Screen::Send, Screen::Send, set.clone(), w.clone());
        assert_eq!(kept, set);
        // Leaving Send forgets it.
        assert_eq!(
            back_after(Screen::Send, Screen::Overview, set, w.clone()),
            None
        );
        // Between two screens that are not returning: nothing.
        assert_eq!(
            back_after(Screen::Overview, Screen::Activity, None, w),
            None
        );
    }

    /// AC-T2: back to the place while it is there; another wallet, identity, a screen this
    /// network does not have, or none — Overview.
    #[test]
    fn back_goes_to_the_place_or_overview() {
        let w = here("w1", Some("id1"));
        let back = back_after(Screen::Activity, Screen::Send, None, w.clone());
        assert_eq!(back_to(back.as_ref(), &w, Testnet), Screen::Activity);
        assert_eq!(back_to(None, &w, Testnet), Screen::Overview);
        assert_eq!(
            back_to(back.as_ref(), &here("w2", Some("id1")), Testnet),
            Screen::Overview,
            "another wallet"
        );
        assert_eq!(
            back_to(back.as_ref(), &here("w1", Some("id2")), Testnet),
            Screen::Overview,
            "another identity"
        );
        let apps = back_after(Screen::Apps, Screen::Send, None, w.clone());
        assert_eq!(back_to(apps.as_ref(), &w, Testnet), Screen::Apps);
        assert_eq!(
            back_to(apps.as_ref(), &w, Mainnet),
            Screen::Overview,
            "not on this network"
        );
        let welcome = back_after(Screen::Welcome, Screen::Send, None, w.clone());
        assert_eq!(back_to(welcome.as_ref(), &w, Testnet), Screen::Overview);
    }
}
