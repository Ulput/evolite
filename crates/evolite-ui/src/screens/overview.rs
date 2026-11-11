//! 02 · Overview: the total, four balance cards, quick actions and the latest operations.

use super::{Screen, txt};
use crate::components::{
    Assets, ButtonKind, CardAmount, GridSpec, TxRow, amount_label, balance_card_in, button,
    button_enabled, chip, grid, grid_layout, page_header, status_chip, tile_amount_font,
    tx_row_clickable,
};
use crate::context::Ctx;
use crate::text::{self};
use crate::theme::{TILE_MIN_W, Weight, mono};
use egui::{Align, Layout, Ui, vec2};
use evolite_api::{Duffs, Proof, Source, Timestamp};

/// 02 · Overview: header, total, four balance cards, quick actions, recent activity.
pub fn overview(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let p = a.p;
    let wallet = cx.wallet();
    let b = &wallet.balances;
    let synced = text::coins_known(cx.snap.sync.stage);
    // The Core balance not checked in this sync: the last checked one, dim, with its time, or
    // none — «Total» counts it, so it is as sure (BUG-77).
    let as_of = text::core_as_of(b.core_as_of, cx.now);
    let unknown = b.core_as_of == Some(evolite_api::CoreAsOf::NotYet);
    // The private pool not read in this run yet (BUG-66): «Total» counts it as its tile shows
    // it. One line under «Total» says what it does not know yet, Core's and the pool's together.
    let pool = pool_amount(b.shielded, wallet.pool.checked_at);
    // A real core built without the pool never reads it (review r1, should 1); the demo does.
    let pool_read = cfg!(feature = "pool") || crate::is_demo(cx.snap);
    let total_line = if b.platform_known() {
        total_line(as_of.as_deref(), unknown, pool, pool_read)
    } else {
        Some(tr!("Not known yet").to_owned())
    };
    let total_color = if total_line.is_some() {
        p.muted
    } else {
        p.text
    };
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
    total_block(ui, a, cx, wallet, &total_line, total_color, synced);
    balance_cards(ui, a, wallet, pool, &as_of, unknown);
    quick_actions(ui, a, cx, wallet);
    latest_operations(ui, a, cx, wallet, synced);
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

/// «Total», what it does not know yet, and how settled the payments are.
fn total_block(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &Ctx,
    wallet: &evolite_api::Wallet,
    total_line: &Option<String>,
    total_color: egui::Color32,
    synced: bool,
) {
    let p = a.p;
    let b = &wallet.balances;
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
        // «Total» and the eye that hides every amount of the app (FEAT-0007).
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.label(txt(tr!("Total"), 13.0, Weight::Medium, p.muted));
            crate::components::amount_toggle(ui, a, false);
        });
        // A part not known yet leaves no sum to say, and no rate to apply to one (BUG-91).
        let sum = CardAmount::total(b).known();
        let total = sum.map_or_else(|| "—".to_owned(), crate::format::amount);
        amount_label(ui, &total, mono(44.0, Weight::Medium), total_color);
        if let Some(line) = &total_line {
            ui.label(txt(line, 13.0, Weight::Regular, p.muted));
        }
        // "Fiat rate": the mainnet price, hidden with the amount.
        if let Some(fiat) = sum.and_then(crate::format::fiat) {
            let line = if !crate::format::display().hide {
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
}

/// The four balance cards.
fn balance_cards(
    ui: &mut Ui,
    a: &mut Assets,
    wallet: &evolite_api::Wallet,
    pool: CardAmount,
    as_of: &Option<String>,
    unknown: bool,
) {
    let b = &wallet.balances;
    // The credits of every identity of the wallet, as «Total» counts them (BUG-56); the one
    // worked as has its own on Identity.
    let several = wallet.identities.len() > 1;
    let credits_title = match wallet.identity.as_ref().and_then(|i| i.name()) {
        Some(n) if !several => format!(tr!("Credits · @{n}"), n = n),
        _ => tr!("Credits").to_string(),
    };
    let unit = crate::format::unit_name();
    let addresses = format!(
        "{unit} · {}",
        text::plural(b.core_addresses, tr!("address"), tr!("addresses"))
    );
    // Not checked in this sync: when it was, or that it is not known yet (BUG-77).
    let core_line = match &as_of {
        Some(line) if !unknown => format!("{unit} · {line}"),
        Some(line) => line.clone(),
        None => addresses,
    };
    let core = CardAmount::core(b);
    let credits_line = if several {
        format!(
            "{unit} · {}",
            text::plural(
                wallet.identities.len() as u32,
                tr!("identity"),
                tr!("identities")
            )
        )
    } else {
        format!(tr!("{unit} in Platform credits"), unit = unit)
    };
    // Not checked in this run: no name and no count either, they may change once it is (BUG-115).
    let credits = credits_amount(b);
    let (credits_title, credits_line) = match credits {
        CardAmount::Now(_) => (credits_title.as_str(), credits_line),
        CardAmount::Earlier(_) => (
            tr!("Credits"),
            format!(tr!("{unit} · not checked yet"), unit = unit),
        ),
        CardAmount::Unknown => (tr!("Credits"), tr!("Not known yet").to_owned()),
    };
    let pool_detail = match pool {
        CardAmount::Now(_) => format!(tr!("{unit} · only you see it"), unit = unit),
        CardAmount::Earlier(_) => format!(tr!("{unit} · not checked yet"), unit = unit),
        CardAmount::Unknown => tr!("Not known yet").to_owned(),
    };
    let address_line = &*if b.platform_address_read {
        format!(tr!("{unit} · no identity"), unit = unit)
    } else {
        tr!("Not known yet").to_owned()
    };
    let cards: [(&str, &str, CardAmount, &str); 4] = [
        ("core", "Core", core, &core_line),
        ("credits", credits_title, credits, &credits_line),
        (
            "platform-address",
            tr!("Platform address"),
            CardAmount::platform(b.platform_address_read, b.platform_address),
            address_line,
        ),
        ("shielded", tr!("Private pool"), pool, &pool_detail),
    ];
    // Four cards in a row while each is at least a tile wide, else two by two, else one under
    // another (responsive spec R4); a row's cards are one height in the first frame.
    let spec = GridSpec {
        min_w: TILE_MIN_W,
        gap: vec2(16.0, 16.0),
        max_cols: 4,
        even: true,
    };
    // One amount size for each row, 24 or 20, decided before the row is drawn (the designer,
    // 2026-11-07): the cells as `grid` lays them out in this very width.
    let (cols, cell_w) = grid_layout(ui.available_width(), cards.len(), spec);
    let fonts: Vec<egui::FontId> = cards
        .chunks(cols)
        .map(|row| {
            let amounts: Vec<CardAmount> = row.iter().map(|c| c.2).collect();
            tile_amount_font(ui, &amounts, cell_w)
        })
        .collect();
    grid(ui, cards.len(), spec, |ui, i, w, cell| {
        let (icon, title, amount, detail) = &cards[i];
        let font = fonts[i / cols].clone();
        balance_card_in(ui, a, cell, (icon, title), (*amount, font), detail, w);
    });
}

/// Top up, shield, withdraw.
fn quick_actions(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, wallet: &evolite_api::Wallet) {
    let b = &wallet.balances;
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
}

/// The latest operations, and the way to all of them.
fn latest_operations(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    wallet: &evolite_api::Wallet,
    synced: bool,
) {
    let p = a.p;
    let b = &wallet.balances;
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
                // Platform's operations count too: «none yet» waits for the end of its search.
                if synced && b.credits_read {
                    tr!("No operations yet. Receive DASH on the “Receive” screen.")
                } else {
                    tr!("Still reading the operations — they appear here as soon as they are found")
                },
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
                    alert: text::activity_alert(act),
                    quiet: text::credits_quiet(act),
                    core_chip: text::core_chip(act),
                    note: act.note.as_deref(),
                    fiat: None,
                },
            );
            if row.clicked() {
                super::activity::open_card(ui.ctx(), act.id);
            }
        }
    });
}

/// The credits' tile (BUG-115), by the pool's rule: checked in this run, the amount; kept from
/// an earlier run, that amount dim; nothing known, or a kept 0, none.
pub(super) fn credits_amount(b: &evolite_api::Balances) -> CardAmount {
    match (b.credits_read, b.credits_checked, b.all_credits) {
        (true, true, duffs) => CardAmount::Now(duffs),
        (true, false, duffs) if duffs > 0 => CardAmount::Earlier(duffs),
        _ => CardAmount::Unknown,
    }
}

/// The private pool's tile (BUG-66, the designer's improvement 1): read in this run
/// (`checked_at`), its amount; not read yet, the amount the pool's store already gives (dim),
/// or none — a 0 before the first read is not known, it is not there yet.
pub(super) fn pool_amount(shielded: Duffs, checked_at: Option<Timestamp>) -> CardAmount {
    match (checked_at, shielded) {
        (Some(_), duffs) => CardAmount::Now(duffs),
        (None, 0) => CardAmount::Unknown,
        (None, duffs) => CardAmount::Earlier(duffs),
    }
}

/// The one line under «Total» (BUG-77, BUG-66): what it does not know yet, or nothing. `core`:
/// Core's line ([`text::core_as_of`]), `core_unknown` when it is «Not known yet» — then «Total»
/// is «—» and that line alone says why, whatever the pool. Otherwise Core's time and the pool
/// not read yet go into one line. A build without the pool (`pool_built` false: a developer's
/// build) never reads it: the pool is left out of the line and does not dim «Total» (review
/// batch-2026-11-07 r1, should 1).
fn total_line(
    core: Option<&str>,
    core_unknown: bool,
    pool: CardAmount,
    pool_built: bool,
) -> Option<String> {
    let pool = if pool_built { pool } else { CardAmount::Now(0) };
    if core_unknown {
        return core.map(str::to_owned);
    }
    Some(match (core, pool) {
        (core, CardAmount::Now(_)) => return core.map(str::to_owned),
        (None, CardAmount::Earlier(_)) => tr!("The private pool in it is not checked yet").into(),
        (None, CardAmount::Unknown) => {
            tr!("Not counting the private pool: it is still being read").into()
        }
        (Some(core), CardAmount::Earlier(_)) => {
            format!(tr!("{} · the private pool in it is not checked yet"), core)
        }
        (Some(core), CardAmount::Unknown) => format!(
            tr!("{} · not counting the private pool: it is still being read"),
            core
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-115: credits kept from an earlier run are dim until checked; nothing known, or a kept 0,
    /// is not known.
    #[test]
    fn credits_kept_from_before_are_dim_until_checked() {
        let b = |read, checked, all_credits| evolite_api::Balances {
            core: 0,
            core_as_of: None,
            core_addresses: 0,
            credits: all_credits,
            all_credits,
            platform_address: 0,
            shielded: 0,
            credits_read: read,
            credits_checked: checked,
            platform_address_read: true,
        };
        assert_eq!(credits_amount(&b(true, true, 47)), CardAmount::Now(47));
        assert_eq!(credits_amount(&b(true, true, 0)), CardAmount::Now(0));
        assert_eq!(credits_amount(&b(true, false, 47)), CardAmount::Earlier(47));
        assert_eq!(credits_amount(&b(true, false, 0)), CardAmount::Unknown);
        assert_eq!(credits_amount(&b(false, false, 47)), CardAmount::Unknown);
    }

    /// BUG-66: no 0 for the pool before it is read; what the store has is shown dim.
    #[test]
    fn the_pool_is_not_zero_before_it_is_read() {
        assert_eq!(pool_amount(0, None), CardAmount::Unknown);
        assert_eq!(
            pool_amount(48_885_860, None),
            CardAmount::Earlier(48_885_860)
        );
        assert_eq!(pool_amount(0, Some(1_793_000_000)), CardAmount::Now(0));
        assert_eq!(
            pool_amount(48_885_860, Some(1_793_000_000)),
            CardAmount::Now(48_885_860)
        );
    }

    /// BUG-66 with BUG-77: Core and the pool both unchecked — one line under «Total», not two;
    /// Core not known at all — «Total» is «—» and only Core's «Not known yet» is said.
    #[test]
    fn total_has_one_line_for_core_and_the_pool() {
        let unknown = CardAmount::Unknown;
        let stored = CardAmount::Earlier(1);
        let read = CardAmount::Now(1);
        assert_eq!(total_line(None, false, read, true), None);
        assert_eq!(
            total_line(Some("as of 20:07"), false, read, true).as_deref(),
            Some("as of 20:07")
        );
        assert_eq!(
            total_line(None, false, unknown, true).as_deref(),
            Some("Not counting the private pool: it is still being read")
        );
        assert_eq!(
            total_line(None, false, stored, true).as_deref(),
            Some("The private pool in it is not checked yet")
        );
        assert_eq!(
            total_line(Some("as of 20:07"), false, unknown, true).as_deref(),
            Some("as of 20:07 · not counting the private pool: it is still being read")
        );
        assert_eq!(
            total_line(Some("as of 20:07"), false, stored, true).as_deref(),
            Some("as of 20:07 · the private pool in it is not checked yet")
        );
        for pool in [unknown, stored, read] {
            assert_eq!(
                total_line(Some("Not known yet"), true, pool, true).as_deref(),
                Some("Not known yet")
            );
        }
    }

    /// Review batch-2026-11-07 r1, should 1: a build without the pool never reads it — the
    /// line under «Total» leaves the pool out (none at all, or Core's alone), so «Total» is not
    /// dimmed for it; Core not known yet is still said.
    #[test]
    fn without_the_pool_built_total_does_not_wait_for_it() {
        for pool in [
            CardAmount::Unknown,
            CardAmount::Earlier(1),
            CardAmount::Now(1),
        ] {
            assert_eq!(total_line(None, false, pool, false), None);
            assert_eq!(
                total_line(Some("as of 20:07"), false, pool, false).as_deref(),
                Some("as of 20:07")
            );
            assert_eq!(
                total_line(Some("Not known yet"), true, pool, false).as_deref(),
                Some("Not known yet")
            );
        }
    }
}
