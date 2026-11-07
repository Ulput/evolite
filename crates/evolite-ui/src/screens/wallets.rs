//! · the wallets of this computer: the switcher under the logo, its menu (Figma 26) and
//! the lines the menu, Settings → Wallets (Figma 27) and the unlock screen say about a wallet.

use crate::components::{Assets, paint_wallet_dot};
use crate::context::{Ctx, WalletRequest};
use crate::text;
use crate::theme::{Weight, sans};
use chrono::{DateTime, Local};
use egui::{Align2, CornerRadius, CursorIcon, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use evolite_api::{Snapshot, WalletEntry, WalletOrigin};

/// The name a wallet the core added without one gets, in the interface language of the
/// moment: "Main" for the first in the list, "Wallet N" for the others. Kept from then on
/// like a name the person gives.
pub fn default_name(place: usize) -> String {
    default_name_in(crate::i18n::current(), place)
}

/// The default name in place of an empty one, for the screen only (the index is untouched).
pub fn show_default_names(wallets: &mut [WalletEntry]) {
    for (place, w) in wallets.iter_mut().enumerate() {
        if w.name.is_empty() {
            w.name = default_name(place);
        }
    }
}

fn default_name_in(lang: crate::i18n::Lang, place: usize) -> String {
    let name = if place == 0 {
        crate::i18n::t_in(lang, tr!("Main")).into_owned()
    } else {
        crate::i18n::t_in(lang, &format!(tr!("Wallet {}"), place + 1)).into_owned()
    };
    // What the core takes: 1–24 characters.
    name.chars().take(24).collect()
}

/// "wallet 1 of 3": the open one's place in the list.
pub fn place_line(snap: &Snapshot) -> String {
    let n = snap.wallets.len();
    let i = snap
        .wallets
        .iter()
        .position(|w| w.open)
        .map_or(n, |i| i + 1);
    format!(tr!("wallet {i} of {n}"), i = i, n = n)
}

/// The menu's line under a wallet: "open · 12,8460 DASH", "locked · opened yesterday".
pub fn menu_line(w: &WalletEntry, snap: &Snapshot, now: DateTime<Local>) -> String {
    if w.open {
        match &snap.wallet {
            // The total counts Core: «—» while Core is not known yet, not a 0 (BUG-80).
            Some(wallet) => format!(
                tr!("open · {}"),
                crate::components::CardAmount::total(&wallet.balances).balance()
            ),
            None => tr!("open").into(),
        }
    } else {
        match w.last_opened {
            Some(at) => format!(tr!("locked · opened {}"), text::when(at, now)),
            None => tr!("locked").into(),
        }
    }
}

/// Settings → Wallets: "testnet · created 12 October 2026 · identity @satoshi" for the open
/// one, "… · last sign-in yesterday" for the others.
pub fn settings_line(w: &WalletEntry, snap: &Snapshot, now: DateTime<Local>) -> String {
    let network = match snap.network {
        evolite_api::Network::Testnet => tr!("Testnet"),
        evolite_api::Network::Mainnet => tr!("Mainnet"),
    };
    // Whole sentences with the date in them: "created" alone and the date beside it stayed
    // half English in a translated line.
    let date = text::date_long(w.created_at);
    let made = match w.origin {
        WalletOrigin::Created => format!(tr!("created {}"), date),
        WalletOrigin::Restored => format!(tr!("restored from a phrase {}"), date),
        WalletOrigin::DashPay => format!(tr!("imported from DashPay {}"), date),
    };
    // Each part translated on its own and the line marked as done: as one text it was taken
    // for a shorter template and came out half translated ("last opened yesterday").
    let lang = crate::i18n::current();
    let part = |s: &str| crate::i18n::t_in(lang, s).into_owned();
    let mut parts = vec![part(network), part(&made)];
    if w.open {
        let name = snap
            .wallet
            .as_ref()
            .and_then(|wallet| wallet.identity.as_ref())
            .and_then(|i| i.names.iter().find(|n| n.primary).or(i.names.first()));
        if let Some(name) = name {
            parts.push(part(&format!(
                tr!("identity {}"),
                format!("@{}", name.label)
            )));
        }
    } else if let Some(at) = w.last_opened {
        parts.push(part(&format!(tr!("last opened {}"), text::when(at, now))));
    }
    crate::i18n::own(&parts.join(" · "))
}

/// The switcher under the logo (Figma 26 "Wallet switcher", 215 × 50): the open wallet's
/// circle, its name and place; in the rail only the circle. Opens and closes the menu.
pub fn switcher(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, rail: bool, menu: &mut bool) {
    let p = a.p;
    let snap = cx.snap;
    let Some(open) = snap.wallets.iter().find(|w| w.open) else {
        return;
    };
    let height = if rail { 44.0 } else { 50.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let own = crate::i18n::own(&open.name);
    crate::components::named(&resp, egui::WidgetType::Button, true, &own);
    let fill = if resp.hovered() { p.raised } else { p.surface };
    let stroke = if *menu { p.brand } else { p.line };
    if !rail || *menu || resp.hovered() {
        ui.painter().rect(
            rect,
            CornerRadius::same(12),
            fill,
            Stroke::new(1.0, stroke),
            StrokeKind::Inside,
        );
    }
    let dot = if rail {
        Rect::from_center_size(rect.center(), vec2(28.0, 28.0))
    } else {
        Rect::from_min_size(
            pos2(rect.left() + 10.0, rect.center().y - 14.0),
            vec2(28.0, 28.0),
        )
    };
    paint_wallet_dot(ui, dot, open);
    if !rail {
        let x = dot.right() + 10.0;
        let width = rect.right() - 36.0 - x;
        let name =
            ui.painter()
                .layout_no_wrap(open.name.clone(), sans(14.0, Weight::SemiBold), p.text);
        let name = elide(
            ui,
            name,
            &open.name,
            width,
            sans(14.0, Weight::SemiBold),
            p.text,
        );
        ui.painter().galley(pos2(x, rect.top() + 9.0), name, p.text);
        crate::components::paint_text(
            ui,
            pos2(x, rect.top() + 29.0),
            Align2::LEFT_TOP,
            &place_line(snap),
            sans(11.0, Weight::Regular),
            p.muted,
        );
        a.icons.icon("chevron-down", p.muted, &p, 16.0).paint_at(
            ui,
            Rect::from_center_size(pos2(rect.right() - 20.0, rect.center().y), vec2(16.0, 16.0)),
        );
    }
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let resp = if rail {
        resp.on_hover_text(open.name.clone())
    } else {
        resp
    };
    if resp.clicked() {
        *menu = !*menu;
    }
    if *menu {
        wallet_menu(ui, a, cx, rect, menu);
    }
}

/// A wallet's name cut with "…" to `width` (names are the person's, up to 24 characters).
fn elide(
    ui: &Ui,
    galley: std::sync::Arc<egui::Galley>,
    name: &str,
    width: f32,
    font: egui::FontId,
    color: egui::Color32,
) -> std::sync::Arc<egui::Galley> {
    if galley.size().x <= width {
        return galley;
    }
    let chars: Vec<char> = name.chars().collect();
    for n in (1..chars.len()).rev() {
        let cut: String = chars[..n].iter().collect::<String>() + "…";
        let g = ui.painter().layout_no_wrap(cut, font.clone(), color);
        if g.size().x <= width {
            return g;
        }
    }
    galley
}

/// The menu under the switcher (Figma 26 "Wallet menu", 320 wide): every wallet, the open
/// one ticked; a locked one opens at its unlock screen. "Add wallet", "Manage
/// wallets" and why the other one asks for its password.
fn wallet_menu(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, anchor: Rect, menu: &mut bool) {
    let p = a.p;
    let snap = cx.snap;
    let now = cx.now;
    let mut choose: Option<String> = None;
    let mut add = false;
    let mut manage = false;
    let area = egui::Area::new(egui::Id::new("wallet-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(anchor.left(), anchor.bottom() + 10.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(egui::Margin::same(9))
                .shadow(egui::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(if p.dark { 120 } else { 40 }),
                })
                .show(ui, |ui| {
                    ui.set_width(302.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let (head, _) = ui.allocate_exact_size(vec2(302.0, 28.0), Sense::hover());
                    crate::components::paint_text(
                        ui,
                        pos2(head.left() + 10.0, head.center().y),
                        Align2::LEFT_CENTER,
                        tr!("WALLETS ON THIS COMPUTER"),
                        sans(11.0, Weight::SemiBold),
                        p.muted,
                    );
                    // The app's one «Hide amounts», as an icon with its words in the tooltip
                    // (FEAT-0007): the open wallet's line below says its balance.
                    let eye = Rect::from_min_size(
                        pos2(head.right() - 4.0 - 28.0, head.top()),
                        vec2(28.0, 28.0),
                    );
                    let mut eye_ui = ui.new_child(egui::UiBuilder::new().max_rect(eye));
                    crate::components::amount_toggle(&mut eye_ui, a, true);
                    for w in &snap.wallets {
                        let (row, resp) = ui.allocate_exact_size(vec2(302.0, 47.0), Sense::click());
                        let own = crate::i18n::own(&w.name);
                        crate::components::named(&resp, egui::WidgetType::Button, true, &own);
                        if w.open || resp.hovered() {
                            ui.painter()
                                .rect_filled(row, CornerRadius::same(10), p.raised);
                        }
                        let dot = Rect::from_min_size(
                            pos2(row.left() + 10.0, row.center().y - 14.0),
                            vec2(28.0, 28.0),
                        );
                        paint_wallet_dot(ui, dot, w);
                        let x = dot.right() + 10.0;
                        let name = ui.painter().layout_no_wrap(
                            w.name.clone(),
                            sans(14.0, Weight::SemiBold),
                            p.text,
                        );
                        let name = elide(
                            ui,
                            name,
                            &w.name,
                            row.right() - 44.0 - x,
                            sans(14.0, Weight::SemiBold),
                            p.text,
                        );
                        ui.painter().galley(pos2(x, row.top() + 7.0), name, p.text);
                        crate::components::paint_text(
                            ui,
                            pos2(x, row.top() + 26.0),
                            Align2::LEFT_TOP,
                            &menu_line(w, snap, now),
                            sans(11.0, Weight::Regular),
                            p.muted,
                        );
                        let (icon, color) = if w.open {
                            ("check", p.accent)
                        } else {
                            ("lock", p.muted)
                        };
                        a.icons.icon(icon, color, &p, 16.0).paint_at(
                            ui,
                            Rect::from_center_size(
                                pos2(row.right() - 18.0, row.center().y),
                                vec2(16.0, 16.0),
                            ),
                        );
                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !w.open {
                            choose = Some(w.id.clone());
                        }
                    }
                    let (line, _) = ui.allocate_exact_size(vec2(302.0, 1.0), Sense::hover());
                    ui.painter().rect_filled(line, 0, p.line);
                    for (icon, label, slot) in [
                        ("plus", tr!("Add wallet"), &mut add),
                        ("wallets", tr!("Manage wallets"), &mut manage),
                    ] {
                        let (row, resp) = ui.allocate_exact_size(vec2(302.0, 36.0), Sense::click());
                        crate::components::named(&resp, egui::WidgetType::Button, true, label);
                        if resp.hovered() {
                            ui.painter()
                                .rect_filled(row, CornerRadius::same(10), p.raised);
                        }
                        a.icons.icon(icon, p.text, &p, 18.0).paint_at(
                            ui,
                            Rect::from_min_size(
                                pos2(row.left() + 14.0, row.center().y - 9.0),
                                vec2(18.0, 18.0),
                            ),
                        );
                        crate::components::paint_text(
                            ui,
                            pos2(row.left() + 42.0, row.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            sans(14.0, Weight::Medium),
                            p.text,
                        );
                        *slot = resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
                    }
                    ui.add_space(2.0);
                    // Wrapped in 282 px, as in Figma: two lines, the menu stays 320 wide.
                    let galley = ui.painter().layout(
                        crate::i18n::t(
                            tr!("Another wallet opens with its own password; the current one gets locked."),
                        )
                        .into_owned(),
                        sans(11.0, Weight::Regular),
                        p.muted,
                        282.0,
                    );
                    let (foot, _) =
                        ui.allocate_exact_size(vec2(302.0, galley.size().y + 4.0), Sense::hover());
                    ui.painter().galley(
                        pos2(foot.left() + 10.0, foot.top() + 4.0),
                        galley,
                        p.muted,
                    );
                    ui.add_space(4.0);
                });
        });
    // A click anywhere else, or Escape, closes the menu (the switcher's own click toggles it).
    let outside = ui.input(|i| {
        i.pointer.any_click()
            && i.pointer
                .interact_pos()
                .is_some_and(|at| !area.response.rect.contains(at) && !anchor.contains(at))
    });
    if outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        *menu = false;
    }
    if let Some(id) = choose {
        cx.wallets(WalletRequest::Open(id));
        *menu = false;
    }
    if add {
        cx.wallets(WalletRequest::Add(None));
        *menu = false;
    }
    if manage {
        cx.wallets(WalletRequest::Manage);
        *menu = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wallet_without_a_name_gets_one_by_its_place() {
        use crate::i18n::Lang;
        assert_eq!(default_name_in(Lang::En, 0), "Main");
        assert_eq!(default_name_in(Lang::En, 2), "Wallet 3");
        for lang in [Lang::En, Lang::Ar] {
            let name = default_name_in(lang, 1);
            assert!((1..=24).contains(&name.chars().count()), "{lang:?}: {name}");
        }
    }

    /// A wallet the core made alone (no name in the index) is shown with its default name,
    /// never as "?"; a name the person gave stays.
    #[test]
    fn an_unnamed_wallet_is_shown_with_its_default_name() {
        let entry = |id: &str, name: &str| WalletEntry {
            id: id.into(),
            name: name.into(),
            color: 0,
            origin: WalletOrigin::Restored,
            created_at: 0,
            last_opened: None,
            open: false,
            wrong_tries: 0,
            wait_until: None,
        };
        let mut wallets = vec![entry("a", ""), entry("b", "Savings")];
        show_default_names(&mut wallets);
        assert_eq!(wallets[0].name, default_name(0));
        assert_ne!(wallets[0].letter(), "?");
        assert_eq!(wallets[1].name, "Savings");
    }
}
