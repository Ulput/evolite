//! BUG-265: the one identity picker — «Work as» on Identity, «Identity» on Tokens and the
//! Workshop. Built from the Identity screen's (design review №14): the label in the middle of
//! the first row of pills; many identities wrap onto more rows, each from under the first pill,
//! never past the page. A name too long for a whole row is shortened in its middle (FEAT-0051
//! R-5); the full one is on hover and in the pill's name for a screen reader. The Collection
//! uses it as a filter (FEAT-0055, [`identity_filter`]): «All» first, then the identities.

use super::prelude::*;
use crate::theme::type_scale::UI;
use evolite_api::{IdentityChoice, Wallet};

/// What the picker was asked this frame.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Picked {
    /// Another identity was clicked: its id.
    pub chosen: Option<String>,
    /// «+ New identity» was clicked.
    pub new_one: bool,
    /// «All» was clicked (only [`identity_filter`] has it).
    pub all: bool,
}

/// The identities of `wallet` as pills after `label`, the shown one on. `new`: `Some(enabled)`
/// adds «+ New identity» at the end (only the Identity screen). `aside`: muted words on the
/// right of the first row (when Tokens were checked, the Workshop's credits).
pub fn identity_picker(
    ui: &mut Ui,
    a: &mut Assets,
    wallet: &Wallet,
    label: &str,
    new: Option<bool>,
    aside: Option<&str>,
) -> Picked {
    let shown = wallet.identity.as_ref().map(|i| i.id.as_str());
    // FEAT-0057 П1 (R-1.1): from five identities, a button and a list with search.
    if wallet.identities.len() >= super::LIST_FROM {
        let p = super::identity_list::identity_list(ui, a, wallet, label, shown, None, new, aside);
        return Picked {
            chosen: p.chosen,
            new_one: p.new_one,
            all: false,
        };
    }
    picker_row(ui, a, &wallet.identities, label, shown, None, new, aside)
}

/// FEAT-0055: the Collection's filter by identity — «All» (on while `only` is `None`), then
/// every identity of `wallet`, `only` on. Clicking the one on changes nothing. `Some(None)`:
/// all now; `Some(Some(id))`: that identity's only.
#[allow(clippy::option_option)]
pub fn identity_filter(
    ui: &mut Ui,
    a: &mut Assets,
    wallet: &Wallet,
    label: &str,
    only: Option<&str>,
) -> Option<Option<String>> {
    // FEAT-0057 П1 (R-1.1): from five identities the list, «All» its first row.
    let picked = if wallet.identities.len() >= super::LIST_FROM {
        let p = super::identity_list::identity_list(
            ui,
            a,
            wallet,
            label,
            only,
            Some(only.is_none()),
            None,
            None,
        );
        Picked {
            chosen: p.chosen,
            new_one: false,
            all: p.all,
        }
    } else {
        picker_row(
            ui,
            a,
            &wallet.identities,
            label,
            only,
            Some(only.is_none()),
            None,
            None,
        )
    };
    if picked.all {
        Some(None)
    } else {
        picked.chosen.map(Some)
    }
}

/// The row both share: `on` the identity on, `all: Some(on)` an «All» pill first.
#[allow(clippy::too_many_arguments)]
fn picker_row(
    ui: &mut Ui,
    a: &mut Assets,
    identities: &[IdentityChoice],
    label: &str,
    shown: Option<&str>,
    all: Option<bool>,
    new: Option<bool>,
    aside: Option<&str>,
) -> Picked {
    let p = a.p;
    let mut picked = Picked::default();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let font = crate::theme::style(UI.weight(Weight::Medium));
        let label_w = text_width(ui, label, font);
        ui.allocate_ui_with_layout(
            vec2(label_w, PILL_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| ui.label(text(label, UI.weight(Weight::Medium), p.muted)),
        );
        let aside_font = sans(13.0, Weight::Regular);
        let aside_w = aside.map_or(0.0, |s| text_width(ui, s, aside_font.clone()) + 16.0);
        let room = (ui.available_width() - aside_w).max(0.0);
        ui.allocate_ui_with_layout(
            vec2(room, 0.0),
            Layout::left_to_right(Align::Min).with_main_wrap(true),
            |ui| {
                ui.set_max_width(room);
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                if let Some(on) = all
                    && pill(ui, &p, tr!("All"), on).clicked()
                    && !on
                {
                    picked.all = true;
                }
                for choice in identities {
                    let on = shown == Some(choice.id.as_str());
                    if identity_pill(ui, &p, choice, on, room).clicked() && !on {
                        picked.chosen = Some(choice.id.clone());
                    }
                }
                if let Some(enabled) = new {
                    picked.new_one = add_pill(
                        ui,
                        a,
                        tr!("New identity"),
                        tr!("Create a new identity"),
                        enabled,
                    )
                    .clicked();
                }
            },
        );
        if let Some(aside) = aside {
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), PILL_HEIGHT),
                Layout::right_to_left(Align::Center),
                |ui| ui.label(txt(aside, 13.0, Weight::Regular, p.muted)),
            );
        }
    });
    picked
}

/// An identity's whole name: «@name», or its id without one.
pub fn identity_full_name(choice: &IdentityChoice) -> String {
    choice
        .name
        .as_ref()
        .map_or_else(|| choice.id.clone(), |n| format!("@{n}"))
}

/// One identity's pill, no wider than `room`: the whole name when it fits, else its middle
/// shortened (FEAT-0051 R-5); on hover the whole name and the id.
fn identity_pill(
    ui: &mut Ui,
    p: &Palette,
    choice: &IdentityChoice,
    on: bool,
    room: f32,
) -> Response {
    let full = identity_full_name(choice);
    let own = crate::i18n::own(&full);
    let fits = |s: &str| pill_width(ui, s) <= room;
    let shown = if fits(&own) {
        own.clone()
    } else {
        crate::text::middle_fit(&own, &fits, crate::text::NAME_ENDS)
    };
    let hover = match &choice.name {
        Some(_) => format!("{own}\n{}", crate::i18n::t(&choice.id)),
        None => crate::i18n::t(&choice.id).into_owned(),
    };
    pill_named(ui, p, &shown, &own, on).on_hover_text(hover)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_name_is_the_name_or_else_the_id() {
        let named = IdentityChoice {
            id: "8rd3Ux7pQfAbCdEfGhJkLmNoPqRsTuVwXyZkV2mJ38R".into(),
            name: Some("werner555".into()),
            slot: 0,
            ..Default::default()
        };
        assert_eq!(identity_full_name(&named), "@werner555");
        let bare = IdentityChoice {
            name: None,
            ..named.clone()
        };
        assert_eq!(identity_full_name(&bare), named.id);
    }
}
