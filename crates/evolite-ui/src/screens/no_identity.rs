//! BUG-262: a screen that needs an identity, in a wallet with none — its empty state, never a
//! button that does nothing (the customer, 2026-11-22). One piece for every such screen (R-1):
//! the title (a heading, read first), one line on what an identity gives here, and the way to
//! one — «Create an identity» (FEAT-0048's private creation), «Continue creating» when one was
//! cut short (R-3), nothing while the phrase's identity is still looked for (R-2).

use crate::components::{Assets, ButtonKind, button};
use crate::context::Ctx;
use crate::screens::Screen;
use crate::theme::Weight;
use egui::Ui;
use evolite_api::{IdentitySearch, Wallet};

/// The screens that need an identity (the card's table).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Needs {
    Contacts,
    Tokens,
    Collection,
    Workshop,
    Apps,
    Identity,
}

impl Needs {
    fn title(self) -> &'static str {
        match self {
            Needs::Contacts => tr!("Contacts appear once you have an identity"),
            Needs::Tokens => tr!("Tokens appear once you have an identity"),
            Needs::Collection => tr!("Your collection appears once you have an identity"),
            Needs::Workshop => tr!("The Workshop works once you have an identity"),
            Needs::Apps => tr!("Apps appear once you have an identity"),
            Needs::Identity => tr!("You have no identity yet"),
        }
    }

    fn line(self) -> &'static str {
        match self {
            Needs::Contacts => {
                tr!("DashPay contacts and payments by @name work on behalf of an identity.")
            }
            Needs::Tokens => tr!("Dash Platform tokens are held by an identity."),
            Needs::Collection => tr!("NFTs are held by an identity."),
            Needs::Workshop => tr!(
                "Collections and tokens are issued on behalf of an identity and paid with its credits."
            ),
            Needs::Apps => tr!("You sign in to Dash apps on behalf of an identity."),
            Needs::Identity => {
                tr!("An identity is your record on the Dash network: a name, keys and credits.")
            }
        }
    }
}

/// The wallet has no identity at all: the chosen one is always there while any is (R-4: the
/// core shows the first when the chosen one is gone).
pub fn has_none(w: &Wallet) -> bool {
    w.identity.is_none() && w.identities.is_empty()
}

/// The empty state of `needs` when the wallet has no identity; true: it was drawn, and the screen
/// draws nothing else (no button that cannot act, A1, A5).
pub fn empty_state(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, needs: Needs) -> bool {
    draw(ui, a, cx, needs, true)
}

/// [`empty_state`] under the page's own header — the header with no buttons — for the screens
/// whose header holds actions an identity is needed for.
pub fn page(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, title: &str, needs: Needs) -> bool {
    if !has_none(cx.wallet()) {
        return false;
    }
    crate::components::page_header(ui, a, title, None, |_, _| {});
    empty_state(ui, a, cx, needs)
}

/// `offer`: with its button; Identity's own «Continue creating» plate stands above it there.
pub(crate) fn draw(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, needs: Needs, offer: bool) -> bool {
    let wallet = cx.wallet();
    if !has_none(wallet) {
        return false;
    }
    let p = a.p;
    // R-2: an identity may yet be found — no «Create», or a second one would be made.
    let searching = wallet.identity_search == IdentitySearch::Searching;
    // R-3: a creation cut short — finished, not started anew (BUG-248, FEAT-0048 UC-3).
    let unfinished =
        wallet.unfinished_identity.is_some() || wallet.unfinished_pool_identity.is_some();
    let mut create = false;
    let mut again = false;
    ui.add_space(24.0);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        let title = if searching {
            tr!("Looking for this phrase's identity on Platform…")
        } else {
            needs.title()
        };
        let resp = ui.label(super::txt(title, 20.0, Weight::SemiBold, p.text));
        // Read first, as the screen's heading.
        ui.ctx().accesskit_node_builder(resp.id, |node| {
            node.set_role(egui::accesskit::Role::Heading);
        });
        ui.add(crate::components::para(
            needs.line(),
            14.0,
            Weight::Regular,
            p.muted,
        ));
        if searching || (unfinished && !offer) {
            return;
        }
        // r1 Should 1 (§8d, the third state): the search failed — an identity may still be
        // there, so no «Create an identity» (the core would refuse it anyway); why, and «Search
        // again». Identity says it above its own empty state (`search_status`).
        if let IdentitySearch::Failed(why) = &wallet.identity_search
            && !unfinished
        {
            if offer {
                crate::components::notice(
                    ui,
                    a,
                    "warning",
                    p.accent,
                    &format!(
                        tr!(
                            "Could not ask Platform about this phrase's identities ({why}). evolite will try again by itself."
                        ),
                        why = why
                    ),
                );
                again = button(ui, a, tr!("Search again"), ButtonKind::Secondary, None).clicked();
            }
            return;
        }
        let label = if unfinished {
            tr!("Continue creating")
        } else {
            tr!("Create an identity")
        };
        // FEAT-0047 step A («I have an invitation») is not in this build: no button for it (R-6).
        create = button(ui, a, label, ButtonKind::Primary, None).clicked();
    });
    if create {
        cx.go(Screen::CreateIdentity);
    }
    if again {
        cx.send(evolite_api::Command::FindIdentities);
    }
    true
}
