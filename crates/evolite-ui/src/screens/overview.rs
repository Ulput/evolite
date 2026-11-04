//! 02 · Overview: the total, four balance cards, quick actions and the latest operations.

use super::{Screen, txt};
use crate::components::{
    Assets, ButtonKind, TxRow, amount_label, balance_card, button, button_enabled, chip,
    page_header, status_chip, tx_row_clickable,
};
use crate::context::Ctx;
use crate::text::{self};
use crate::theme::{Weight, mono};
use egui::{Align, Layout, Ui, vec2};
use evolite_api::{Proof, Source};

/// 02 · Overview: header, total, four balance cards, quick actions, recent activity.
pub fn overview(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let p = a.p;
    let wallet = cx.wallet();
    let b = &wallet.balances;
    let synced = text::coins_known(cx.snap.sync.stage);
    ui.spacing_mut().item_spacing.y = 22.0;
    page_header(ui, a, tr!("Overview"), None, |ui, a| {
        if button_enabled(ui, a, tr!("Send"), ButtonKind::Primary, None, synced)
            .on_disabled_hover_text(crate::i18n::t(text::WAIT_FOR_SYNC).into_owned())
            .clicked()
        {
            cx.go(Screen::Send);
        }
        if button(ui, a, tr!("Receive"), ButtonKind::Secondary, None).clicked() {
            cx.go(Screen::Receive);
        }
    });
    // Locked or proved: a payment in the mempool, on its way, or in a block without a ChainLock
    // is not settled yet.
    let all_proved = wallet.activity.iter().all(|act| {
        matches!(
            act.proof,
            Proof::InstantLock | Proof::ChainLock | Proof::Proved
        )
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(txt(tr!("Total"), 13.0, Weight::Medium, p.muted));
        let total = amount_label(
            ui,
            &crate::format::amount(b.total()),
            mono(44.0, Weight::Medium),
            p.text,
        );
        // "Fiat rate": the mainnet price, hidden with the amount.
        if let Some(fiat) = crate::format::fiat(b.total()) {
            let shown = !crate::format::display().hide || total.contains_pointer();
            let line = if shown {
                format!(tr!("{fiat} · at the mainnet rate"), fiat = fiat)
            } else {
                "••••".into()
            };
            ui.label(txt(&line, 13.0, Weight::Regular, p.muted));
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            // While the sync runs the wallet does not know all its coins: nothing is final.
            if !synced {
                chip(ui, a, "pending", tr!("Syncing"), false).on_hover_text(
                    crate::i18n::t(tr!("The balance may be incomplete until the sync finishes"))
                        .into_owned(),
                );
                ui.label(txt(
                    tr!("The balance may be incomplete until the sync finishes"),
                    13.0,
                    Weight::Regular,
                    p.muted,
                ));
                return;
            }
            let updated = cx.now.format("%H:%M");
            let (proof, line) = if all_proved {
                (
                    Proof::ChainLock,
                    format!(tr!("All confirmed · updated {updated}"), updated = updated),
                )
            } else {
                (
                    Proof::Mempool,
                    format!(
                        tr!("Some payments are unconfirmed · updated {updated}"),
                        updated = updated
                    ),
                )
            };
            status_chip(ui, a, proof);
            ui.label(txt(&line, 13.0, Weight::Regular, p.muted));
        });
    });
    let credits_title = wallet.identity.as_ref().and_then(|i| i.name()).map_or_else(
        || tr!("Credits").to_string(),
        |n| format!(tr!("Credits · @{n}"), n = n),
    );
    let unit = crate::format::unit_name();
    let addresses = format!(
        "{unit} · {}",
        text::plural(b.core_addresses, tr!("address"), tr!("addresses"))
    );
    let cards: [(&str, &str, i64, &str); 4] = [
        ("core", "Core", b.core, &addresses),
        (
            "credits",
            &credits_title,
            b.credits,
            &format!(tr!("{unit} in Platform credits"), unit = unit),
        ),
        (
            "platform-address",
            tr!("Platform address"),
            b.platform_address,
            &format!(tr!("{unit} · no identity"), unit = unit),
        ),
        (
            "shielded",
            tr!("Private pool"),
            b.shielded,
            &format!(tr!("{unit} · only you see it"), unit = unit),
        ),
    ];
    // Four cards in a row when there is room (≥ 896 px of content), otherwise two.
    let width = ui.available_width();
    let per_row = if width >= 896.0 { 4 } else { 2 };
    let card_w = (width - 16.0 * (per_row - 1) as f32) / per_row as f32;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 16.0;
        for row in cards.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                for (icon, title, duffs, detail) in row {
                    balance_card(ui, a, icon, title, *duffs, detail, card_w);
                }
            });
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        // The top-up window lives on the Identity screen, next to the credits it fills: it
        // opens there at once.
        if button(
            ui,
            a,
            tr!("Top up the identity"),
            ButtonKind::Secondary,
            None,
        )
        .clicked()
        {
            cx.top_up();
        }
        if button(
            ui,
            a,
            tr!("Shield into the pool"),
            ButtonKind::Secondary,
            None,
        )
        .clicked()
        {
            cx.go(Screen::Shielded);
        }
        // Credits back to Core: the Send screen from "Credits" to this wallet's own address
        // (another can be typed there).
        let can_withdraw = wallet.identity.is_some() && b.credits > 0;
        if button_enabled(
            ui,
            a,
            tr!("Withdraw to Core"),
            ButtonKind::Secondary,
            None,
            can_withdraw,
        )
        .on_disabled_hover_text(
            crate::i18n::t(tr!(
                "Nothing to withdraw: an identity with credits is needed"
            ))
            .into_owned(),
        )
        .clicked()
        {
            cx.pay(Source::Credits, wallet.core_address.clone());
        }
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 41.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(txt(
                    tr!("Latest operations"),
                    16.0,
                    Weight::SemiBold,
                    p.text,
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, a, tr!("All history"), ButtonKind::Ghost, None).clicked() {
                        cx.go(Screen::Activity);
                    }
                });
            },
        );
        ui.add_space(2.0);
        if wallet.activity.is_empty() {
            ui.label(txt(
                tr!("No operations yet. Receive DASH on the «Receive» screen."),
                14.0,
                Weight::Regular,
                p.muted,
            ));
        }
        for act in wallet.activity.iter().take(5) {
            let detail = format!(
                "{} · {}",
                text::chain(act.chain),
                text::when(act.at, cx.now)
            );
            // The same card as in History.
            let row = tx_row_clickable(
                ui,
                a,
                &TxRow {
                    icon: text::activity_icon(act),
                    title: &text::activity_title_in(act, &wallet.address_book),
                    detail: &detail,
                    proof: Some(act.proof),
                    amount: &text::signed_amount(act.amount),
                },
            );
            if row.clicked() {
                super::activity::open_card(ui.ctx(), act.id);
            }
        }
    });
    super::activity::card_window(ui, a, cx);
    #[cfg(demo_core)]
    if cx.snap.demo {
        crate::components::demo_line(
            ui,
            a,
            tr!(
                "Demo data: there is no wallet behind these screens, it is the example from the mockup."
            ),
        );
    }
}
