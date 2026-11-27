//! FEAT-0048: an identity created privately, from the pool — the default on both levels (R-1).
//!
//! - The pool holds enough (UC-1): the name, the exit (0.03, 0.25 for a contested name, R-2),
//!   what is private and what is seen (R-5), «Create privately» → the review with the password →
//!   `Spend::CreateIdentityFromPool`; its steps said as they are, never a bar stuck at 95 %.
//! - Too little in it (UC-2): «First put DASH into the pool» with 0.1 (R-6), the pool's own
//!   «Shield» window over this screen; once it is in, the advice on waiting (R-7) and «Create now».
//! - Its first check, minutes long (BUG-245): said, with what to expect.
//! - Sent and not known yet (UC-3) or refused (UC-4): the plates, nothing sent again (BUG-243).
//! - From Core (UC-5): the advanced level's choice, under the warning; and for everyone when the
//!   pool is unavailable (UC-6), with a line on when private creation comes back.
//! - An invitation: an explicit choice that is not private (FEAT-0047, only said for now).

use super::{CreateIdentityForm, NameFacts, dash, name_section};
use crate::components::{
    Assets, ButtonKind, Part, StepState, button, button_enabled, button_row, button_row_spend,
    card, divider, error_notice, inner, kv_row, notice, numbered_step_at,
};
use crate::context::Ctx;
use crate::screens::mixed_lock::{self, MixedLock};
use crate::screens::shielded::{PoolMove, PoolState};
use crate::theme::{Weight, mono};
use egui::{Margin, Ui};
use evolite_api::{CoreError, Duffs, MixedTo, NameCheck, Spend, UnfinishedPoolIdentity, Wallet};

/// The exit for an ordinary name or none (R-2): 0.03 DASH.
pub const EXIT: Duffs = 3_000_000;
/// The exit for a contested name (И5): 0.25 DASH.
pub const EXIT_CONTESTED: Duffs = 25_000_000;
/// BUG-319 r1: what the pool must hold to create an identity with `exit` — the exit itself (the
/// fee is taken out of it, not from other notes).
pub(crate) fn pool_needs(exit: Duffs) -> Duffs {
    exit
}

/// What «First put DASH into the pool» puts in (R-6): more than the exit, not equal to it (И2).
pub const INTO_POOL: Duffs = 10_000_000;
/// The proof's timer at its cap (the lead): from here Platform is awaited, not the proof.
const PROOF_DONE: f32 = 0.95;
/// The Core source in the pool's «Shield» window ([`crate::screens::shielded`]).
const FROM_CORE: usize = 2;

/// The private path's own state on the form.
#[derive(Default)]
pub struct PoolPath {
    /// «Create from Core (not private)» chosen on the advanced level (UC-5): the Core form under
    /// its warning.
    pub core: bool,
    /// «I have an invitation» opened: what it is, not private (FEAT-0047).
    pub invitation: bool,
    /// The review «Create privately» is open over the form.
    pub confirming: bool,
    /// The core's timer for the creation (to 0.95): «Building the private proof», then
    /// «Waiting for Platform».
    pub progress: Option<f32>,
    /// «First put DASH into the pool» was used: once the pool holds enough, the advice on
    /// waiting and «Create now» (R-7).
    pub shielded_for_it: bool,
    /// The creation awaited was sent from here: a wrong password opens this review again, not
    /// the Core one.
    pub sent: bool,
    /// UC-4 «Shield again» open over the screen, with what Platform returned (the plate's sum).
    pub refund: Option<Duffs>,
    /// Step 2 (path M): the core's plan for the exit from fully mixed coins.
    pub mixed: MixedLock,
    /// «From mixed coins» or «From the pool» as chosen; `None`: by the order (И6, И8) — the pool
    /// when it holds enough, else mixed coins when they do.
    pub from_mixed: Option<bool>,
    /// The creation awaited was sent from mixed coins: its steps are the Core form's four.
    pub sent_mixed: bool,
}

/// The name as the form has it, worked out by [`super::create_identity`].
pub struct NameNow<'a> {
    pub label: &'a str,
    pub check: &'a Option<Result<NameCheck, CoreError>>,
    pub nameless: bool,
    /// A contested name: the identity is made without it, its claim offered right after (as from
    /// Core, FEAT-0021 UC-5); the exit is 0.25 (И5).
    pub claim: Option<&'a str>,
}

/// What the pool lets this screen do now.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum PoolNow {
    /// Not in this build or network, its keys not open, or its scan failing (UC-6).
    Unavailable,
    /// Being connected: the first check, minutes long (BUG-245).
    Connecting,
    Ready,
}

/// The lead's signs (FEAT-0048 answer 3): no exits offered, a failing scan, no address while not
/// connecting → unavailable; connecting or not read yet → connecting.
pub(crate) fn pool_now(w: &Wallet) -> PoolNow {
    let pool = &w.pool;
    if w.fees.pool_identity_exits.is_empty() || pool.failing.is_some() {
        PoolNow::Unavailable
    } else if pool.connecting || (pool.address.is_some() && pool.checked_at.is_none()) {
        PoolNow::Connecting
    } else if pool.address.is_none() {
        PoolNow::Unavailable
    } else {
        PoolNow::Ready
    }
}

/// The exit for this name (R-2): 0.25 for a contested one, else 0.03 — or the nearest the
/// network offers.
pub(crate) fn exit_for(fees: &evolite_api::Fees, contested: bool) -> Duffs {
    let wanted = if contested { EXIT_CONTESTED } else { EXIT };
    let exits = &fees.pool_identity_exits;
    if exits.contains(&wanted) {
        return wanted;
    }
    exits
        .iter()
        .copied()
        .filter(|e| *e >= wanted)
        .min()
        .or_else(|| exits.iter().copied().max())
        .unwrap_or(wanted)
}

/// UC-5, UC-6, R-8: what creating from Core gives away, before the password.
pub fn core_link_warning() -> &'static str {
    tr!(
        "This identity will be linked to your Core coins for good: anyone will see which addresses paid for it."
    )
}

/// What the Core form says at its top when the private path hands the screen to it (UC-5, UC-6).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CoreBanner {
    /// The pool unavailable: for everyone, with when private creation comes back.
    PoolUnavailable,
    /// The advanced level chose Core: with the way back.
    Chosen,
}

/// The warning at the top of the Core form, in its scroll with it. True: «Create privately
/// instead» was pressed.
pub fn core_banner(ui: &mut Ui, a: &mut Assets, banner: CoreBanner) -> bool {
    let p = a.p;
    notice(ui, a, "warning", p.danger, core_link_warning());
    match banner {
        CoreBanner::PoolUnavailable => {
            notice(
                ui,
                a,
                "info",
                p.muted,
                tr!("Private creation becomes available once the pool works."),
            );
            false
        }
        CoreBanner::Chosen => button(
            ui,
            a,
            tr!("Create privately instead"),
            ButtonKind::Secondary,
            None,
        )
        .clicked(),
    }
}

/// The private path. `None` when it drew the screen; else it hands the screen to the Core form,
/// with what that form says at its top ([`core_banner`]).
pub fn draw(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut CreateIdentityForm,
    shield: &mut PoolState,
    name: &NameNow,
) -> Option<CoreBanner> {
    let handed = draw_path(ui, a, cx, form, shield, name);
    // UC-4: over whatever is under it — the plate goes once the money is back in the pool, the
    // window stays with its answer until «Done».
    if let Some(back) = form.pool.refund {
        refund_dialog(ui, a, cx, form, shield, back);
    }
    handed
}

fn draw_path(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut CreateIdentityForm,
    shield: &mut PoolState,
    name: &NameNow,
) -> Option<CoreBanner> {
    let p = a.p;
    let wallet = cx.wallet();
    // UC-3, UC-4: one sent from the pool comes first — nothing new until its outcome is known.
    // r1: one proved never to reach the network is said, and a new creation may start.
    let not_delivered = wallet
        .unfinished_pool_identity
        .as_ref()
        .is_some_and(|s| s.not_delivered);
    if let Some(sent) = wallet
        .unfinished_pool_identity
        .as_ref()
        .filter(|s| !s.not_delivered)
    {
        // r1 Should 1: no second «Shield again» while one runs, or once it went through and the
        // plate is not gone yet.
        let refunding = shield.flow.waiting()
            || form.pool.refund.is_some()
            || matches!(shield.result, Some((PoolMove::Shield, Ok(()))));
        match unfinished_plate(ui, a, sent, cx.now, !refunding) {
            Some(Plate::Again(back)) => {
                form.pool.refund = Some(back);
                shield.result = None;
            }
            Some(Plate::Back) => cx.go(crate::screens::Screen::Identity),
            None => {}
        }
        return None;
    }
    let advanced = cx.snap.advanced_features;
    let now = pool_now(wallet);
    // UC-6: for everyone, the Core form under the warning, and when the private one comes back.
    if now == PoolNow::Unavailable {
        return Some(CoreBanner::PoolUnavailable);
    }
    // UC-5: the advanced level chose Core: its warning, and the way back.
    if form.pool.core && advanced {
        return Some(CoreBanner::Chosen);
    }
    form.pool.core = false;
    if form.pool.invitation {
        invitation(ui, a, form);
        return None;
    }
    // Made from the pool: what the person signs, worked out once.
    let fees = &wallet.fees;
    let exit = exit_for(fees, name.claim.is_some());
    let fee = fees.pool_identity_create;
    let in_pool = wallet.balances.shielded;
    // BUG-319 r1 Must: the pool needs the exit itself, nothing on top. platform-wallet reserves
    // notes covering the denomination only («NOT denomination + fee — the fee is metered FROM
    // the denomination at execution»: `wallet/shielded/operations.rs`,
    // `identity_create_from_shielded_pool` → `reserve_unspent_notes_for_denomination`), and the
    // identity gets the exit less the fee. `fee` is only the promise «at least ≈» below.
    let needed = pool_needs(exit);
    let enough = in_pool >= needed;
    // Step 2 (path M, И6–И8): the exit from fully mixed coins when there are some; asked once
    // for each exit, sent after the screen is drawn.
    let has_mixed = wallet.balances.mixed.is_some_and(|m| m.spendable > 0);
    let preview = has_mixed
        .then(|| {
            form.pool
                .mixed
                .ask_in(ui.ctx(), mixed_lock::CREATE, MixedTo::Identity, exit)
        })
        .flatten();
    let mixed_ok = has_mixed && form.pool.mixed.plan(MixedTo::Identity, exit).is_some();
    let use_mixed = mixed_ok && form.pool.from_mixed.unwrap_or(!enough);
    let more = wallet.identity.is_some();
    // The claim's own spend comes after; the pool pays its 0.25 (И5), so never «too small».
    form.claim_short = false;
    let name_ok = name.nameless
        || matches!(name.check, Some(Ok(NameCheck::Available)))
        || name.claim.is_some();
    let waiting = form.waiting();
    let ready = if use_mixed {
        name_ok
            && !waiting
            && !shield.flow.waiting()
            && form.pool.mixed.ready(MixedTo::Identity, exit)
    } else {
        now == PoolNow::Ready && enough && name_ok && !waiting && !shield.flow.waiting()
    };
    let ask = crate::components::password_asked(cx.snap) && !waiting;
    let chosen = wallet.identity.as_ref().map(|i| i.id.clone());

    let width = ui.available_width();
    egui::ScrollArea::vertical()
        .id_salt("create-identity-pool")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 24.0;
            card(&p, Margin::same(24)).show(ui, |ui| {
                ui.set_width(inner(width, 24));
                ui.spacing_mut().item_spacing.y = 16.0;
                if not_delivered {
                    notice(
                        ui,
                        a,
                        "info",
                        p.muted,
                        tr!(
                            "It did not reach the network: the money is in the pool, and you can create again."
                        ),
                    );
                }
                let facts = NameFacts {
                    wallet,
                    more,
                    label: name.label,
                    check: name.check,
                    nameless: name.nameless,
                    name_ok,
                };
                name_section(ui, a, form, &facts);
                divider(ui, &p);
                ui.label(super::txt(
                    if use_mixed {
                        tr!("2 · From mixed coins")
                    } else {
                        tr!("2 · From the pool")
                    },
                    15.0,
                    Weight::SemiBold,
                    p.text,
                ));
                // И6: both possible — the pool first, mixed coins the second choice; И8: the pool
                // short, mixed coins by default, no «First put DASH into the pool».
                if mixed_ok && !waiting {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if crate::components::pill(ui, &p, tr!("From the pool"), !use_mixed).clicked() {
                            form.pool.from_mixed = Some(false);
                        }
                        if crate::components::pill(ui, &p, tr!("From mixed coins"), use_mixed).clicked()
                        {
                            form.pool.from_mixed = Some(true);
                        }
                    });
                }
                if use_mixed {
                    mixed_lock::plan_lines(ui, a, &mut form.pool.mixed, MixedTo::Identity, exit);
                    mixed_private_lines(ui, a);
                } else {
                match now {
                    PoolNow::Connecting => notice(
                        ui,
                        a,
                        "pending",
                        p.muted,
                        tr!(
                            "The private pool is connecting. The first pool check can take up to ~10 minutes; then you can create."
                        ),
                    ),
                    _ => {
                        let value = mono(13.0, Weight::Medium);
                        kv_row(ui, &p, tr!("In the pool"), &dash(in_pool), value.clone());
                        kv_row(ui, &p, tr!("From the pool"), &dash(exit), value.clone());
                        // BUG-319: an upper estimate — the network may take less, never shown less.
                        kv_row(
                            ui,
                            &p,
                            tr!("Platform fee"),
                            &format!(tr!("up to ≈ {}"), dash(fee)),
                            value,
                        );
                    }
                }
                if now == PoolNow::Ready && !enough {
                    let core = Core::of(wallet);
                    if first_into_pool(ui, a, form, shield, needed, in_pool, core) {
                        cx.go(crate::screens::Screen::Receive);
                    }
                } else if now == PoolNow::Ready && form.pool.shielded_for_it {
                    // R-7: advice only, never a wait imposed.
                    notice(
                        ui,
                        a,
                        "info",
                        p.muted,
                        tr!(
                            "The longer between putting into the pool and creating, the harder it is to link them."
                        ),
                    );
                }
                private_lines(ui, a, exit);
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let label = if form.pool.shielded_for_it && !use_mixed {
                        tr!("Create now")
                    } else {
                        tr!("Create privately")
                    };
                    if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked() {
                        form.pool.confirming = true;
                        form.result = None;
                    }
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        cx.go(crate::screens::Screen::Identity);
                    }
                });
                // The other ways, each said for what it is: not private.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if button(ui, a, tr!("I have an invitation"), ButtonKind::Ghost, None)
                        .clicked()
                    {
                        form.pool.invitation = true;
                    }
                    // R-1, A5: from Core only on the advanced level.
                    if advanced
                        && button(
                            ui,
                            a,
                            tr!("Create from Core (not private)"),
                            ButtonKind::Ghost,
                            None,
                        )
                        .clicked()
                    {
                        form.pool.core = true;
                    }
                });
                answer(ui, a, form);
            });
            if use_mixed || (waiting && form.pool.sent_mixed) {
                mixed_steps_card(ui, a, form, width, name, exit);
            } else {
                steps_card(ui, a, form, width, name);
            }
        });
    if let Some(command) = preview {
        cx.send(command);
    }

    if shield.shielding {
        crate::screens::shielded::shield_dialog(ui, a, cx, shield);
    }
    if form.pool.confirming {
        let plan = Plan {
            exit,
            fee,
            label: (!name.nameless && name.claim.is_none()).then(|| name.label.to_owned()),
            claim: name.claim.map(str::to_owned),
            mixed: use_mixed,
        };
        review(ui, a, cx, form, &plan, ask, ready, chosen);
    }
    None
}

/// What Core can put into the pool, as far as it is known (BUG-315).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Core {
    /// Checked in this sync: what it holds; the Core payment on top of an amount sent from it;
    /// the Platform fee taken out of that amount on its way into the pool.
    Known {
        has: Duffs,
        core_payment: Duffs,
        shield_fee: Duffs,
    },
    /// Not checked yet: nothing said about it, the «Shield» window checks.
    NotKnown,
}

impl Core {
    fn of(w: &Wallet) -> Self {
        if w.balances.core_as_of.is_none() {
            Core::Known {
                has: w.balances.core,
                core_payment: w.fees.core_payment,
                shield_fee: w.fees.shield_from_core,
            }
        } else {
            Core::NotKnown
        }
    }
}

/// Core is short too (BUG-315): what it has, and how much more must come to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct CoreShort {
    pub has: Duffs,
    pub more: Duffs,
}

/// What «First put DASH into the pool» offers (R-6, BUG-315): 0.1 when Core can send it and it
/// covers the shortfall; less, all Core can send, when that still covers it; `Err`: Core is
/// short too.
///
/// The same model as the «Shield» window (`price.rs`, `Spend::Shield { from: Core }`, and the live
/// test `live_pool_core_and_identity`): for an amount X, Core spends X + `core_payment` and the
/// pool gets X − `shield_fee` — Platform's fee comes out of the amount, not on top (r1 Must).
pub(crate) fn into_pool(short: Duffs, core: Core) -> Result<Duffs, CoreShort> {
    let Core::Known {
        has,
        core_payment,
        shield_fee,
    } = core
    else {
        return Ok(INTO_POOL);
    };
    // The most Core can send: what it has, less its own payment.
    let max = has - core_payment;
    let arrives = |amount: Duffs| amount - shield_fee;
    if INTO_POOL <= max && arrives(INTO_POOL) >= short {
        Ok(INTO_POOL)
    } else if max > 0 && arrives(max) >= short {
        Ok(max)
    } else {
        Err(CoreShort {
            has,
            more: short + shield_fee + core_payment - has,
        })
    }
}

/// UC-2: too little in the pool — the first step, with the pool's own «Shield» window. BUG-315:
/// never an amount Core does not have; Core short too — how much more and «Receive» (`true`:
/// pressed).
fn first_into_pool(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut CreateIdentityForm,
    shield: &mut PoolState,
    needed: Duffs,
    in_pool: Duffs,
    core: Core,
) -> bool {
    let p = a.p;
    let short = needed.saturating_sub(in_pool);
    notice(
        ui,
        a,
        "info",
        p.text,
        &format!(
            tr!("Not enough in the pool: {} are needed, there are {}."),
            dash(needed),
            dash(in_pool)
        ),
    );
    ui.label(super::txt(
        tr!("First put DASH into the pool"),
        14.0,
        Weight::SemiBold,
        p.text,
    ));
    let put = match into_pool(short, core) {
        Ok(put) => put,
        Err(CoreShort { has, more }) => {
            notice(
                ui,
                a,
                "warning",
                p.text,
                &format!(
                    tr!(
                        "Core has only {}: {} more is needed. Receive DASH to Core, then put it into the pool."
                    ),
                    dash(has),
                    dash(more)
                ),
            );
            return button(ui, a, tr!("Receive"), ButtonKind::Secondary, None).clicked();
        }
    };
    ui.add(crate::components::para(
        &format!(
            tr!(
                "It is visible that you put {} into the pool. It is not visible that the identity was created from this money."
            ),
            dash(put)
        ),
        13.0,
        Weight::Regular,
        p.muted,
    ));
    if button(
        ui,
        a,
        &format!(tr!("Put {} into the pool"), dash(put)),
        ButtonKind::Secondary,
        None,
    )
    .clicked()
    {
        shield.shielding = true;
        shield.from = FROM_CORE;
        shield.amount = crate::format::input_text(put);
        shield.suggested = Some(put);
        shield.result = None;
        form.pool.shielded_for_it = true;
    }
    false
}

/// R-5 (A8): what is private and what is seen, before the password. Never «fully anonymous».
fn private_lines(ui: &mut Ui, a: &mut Assets, exit: Duffs) {
    let p = a.p;
    notice(
        ui,
        a,
        "shield",
        p.text,
        &format!(
            tr!(
                "Private: nobody can link this identity to your Core coins. Only that an identity was created from the pool for {} is visible."
            ),
            dash(exit)
        ),
    );
    notice(
        ui,
        a,
        "eye",
        p.muted,
        tr!("Your name, contacts and profile are visible to everyone — you reveal them yourself."),
    );
}

/// Path M (R-5): what is private and what is seen — the coins' history before mixing is not.
fn mixed_private_lines(ui: &mut Ui, a: &mut Assets) {
    let p = a.p;
    notice(
        ui,
        a,
        "shield",
        p.text,
        tr!(
            "Private: the identity is paid with mixed coins — their history before mixing is not visible."
        ),
    );
    notice(
        ui,
        a,
        "eye",
        p.muted,
        tr!("Your name, contacts and profile are visible to everyone — you reveal them yourself."),
    );
}

/// Path M's «What happens»: the Core form's four steps — an asset lock of the denominations, its
/// confirmation, the identity, the name.
fn mixed_steps_card(
    ui: &mut Ui,
    a: &mut Assets,
    form: &CreateIdentityForm,
    width: f32,
    name: &NameNow,
    exit: Duffs,
) {
    let p = a.p;
    let locked = form
        .pool
        .mixed
        .plan(MixedTo::Identity, exit)
        .map_or(exit, |q| q.nominal);
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 14.0;
        ui.label(super::txt(
            tr!("What happens"),
            15.0,
            Weight::SemiBold,
            p.text,
        ));
        super::Steps {
            locked,
            current: form.waiting().then_some(form.step.unwrap_or(1)),
            name: (!name.nameless).then(|| format!("@{}", name.claim.unwrap_or(name.label))),
            claim_fund: None,
        }
        .draw(ui, a);
    });
}

/// The core's answer, as on the Core form.
fn answer(ui: &mut Ui, a: &mut Assets, form: &CreateIdentityForm) {
    let p = a.p;
    match &form.result {
        _ if form.waiting() => {}
        Some(Err(e @ (CoreError::NotAvailable(_) | CoreError::NotSynced))) => {
            notice(ui, a, "info", p.muted, &super::create_error(e))
        }
        Some(Err(e)) => crate::components::error_notice(
            ui,
            a,
            &super::create_error(e),
            crate::text::error_details(e).as_deref(),
        ),
        Some(Ok(())) => notice(ui, a, "confirmed", p.accent, tr!("Identity made.")),
        None => {}
    }
}

/// The step a creation from the pool is on: none before it is sent; the proof until the core's
/// timer reaches its cap ([`PROOF_DONE`]), then Platform — no real progress exists in between
/// (the lead, T-1).
pub(crate) fn pool_step(waiting: bool, progress: Option<f32>) -> Option<usize> {
    waiting.then(|| {
        if progress.is_some_and(|f| f >= PROOF_DONE) {
            2
        } else {
            1
        }
    })
}

/// «What happens», honest (the lead's T-1, BUG-245): the proof is built here (about half a
/// minute), then Platform is awaited (a minute or two) — no bar stuck at 95 %; then the name.
fn steps_card(ui: &mut Ui, a: &mut Assets, form: &CreateIdentityForm, width: f32, name: &NameNow) {
    let p = a.p;
    let current = pool_step(form.waiting(), form.pool.progress);
    let state = |n| StepState::of(n, current);
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 14.0;
        ui.label(super::txt(
            tr!("What happens"),
            15.0,
            Weight::SemiBold,
            p.text,
        ));
        numbered_step_at(
            ui,
            a,
            1,
            tr!("Building the private proof"),
            tr!("On this computer, about half a minute."),
            state(1),
        );
        numbered_step_at(
            ui,
            a,
            2,
            tr!("Waiting for Platform"),
            tr!("Platform checks the proof and makes the identity: usually one to two minutes."),
            state(2),
        );
        let (step, detail) = match (name.claim, name.nameless) {
            (Some(label), _) => (
                format!(tr!("Name {shown_name}"), shown_name = format!("@{label}")),
                tr!("You claim it right after, on Identity").to_string(),
            ),
            (None, true) => (
                tr!("Name, if given").to_string(),
                tr!("without a name this step is skipped").to_string(),
            ),
            (None, false) => (
                format!(
                    tr!("Name {shown_name}"),
                    shown_name = if name.label.is_empty() {
                        tr!("name").to_string()
                    } else {
                        format!("@{}", name.label)
                    }
                ),
                tr!("written to DPNS").to_string(),
            ),
        };
        numbered_step_at(ui, a, 3, &step, &detail, state(3));
    });
}

/// UC-3, UC-4: sent from the pool — not known yet («never failed», BUG-243), or refused with the
/// money on the refusal address. `Some`: «Shield again» or «Back» pressed.
fn unfinished_plate(
    ui: &mut Ui,
    a: &mut Assets,
    sent: &UnfinishedPoolIdentity,
    now: chrono::DateTime<chrono::Local>,
    offer: bool,
) -> Option<Plate> {
    let p = a.p;
    let width = ui.available_width();
    let mut again = None;
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 12.0;
        match sent.refused {
            None => {
                ui.label(super::txt(
                    tr!("The identity from the pool is being checked"),
                    15.0,
                    Weight::SemiBold,
                    p.text,
                ));
                ui.add(crate::components::para(
                    &format!(
                        tr!(
                            "{} went from the pool {}. Nothing is sent again: evolite finds out from Platform whether the identity was made."
                        ),
                        dash(sent.exit),
                        crate::text::date_time_in_line(sent.sent_at, now)
                    ),
                    13.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            Some(back) => {
                notice(
                    ui,
                    a,
                    "warning",
                    p.danger,
                    &format!(
                        tr!("The identity was not created: {} went to your Platform address."),
                        dash(back)
                    ),
                );
                ui.add(crate::components::para(
                    &sent.refusal_address,
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
                // UC-4: one action, from this address back into the pool (`Spend::ShieldRefund`).
                if offer && button(ui, a, tr!("Shield again"), ButtonKind::Primary, None).clicked() {
                    again = Some(Plate::Again(back));
                }
            }
        }
        // The way out, in both states: the screen has no other widget, and Tab must land
        // somewhere (the Tab sweep, every preset).
        if button(ui, a, tr!("Back"), ButtonKind::Ghost, None).clicked() {
            again = Some(Plate::Back);
        }
    });
    again
}

/// What the plate of a creation sent from the pool was asked to do.
enum Plate {
    /// UC-4 «Shield again», for the sum Platform returned.
    Again(Duffs),
    /// «Back» to Identity.
    Back,
}

/// UC-4 «Shield again»: what Platform returned to the refusal address goes back into the pool,
/// less the fee — with the password, as every spend; once done, its answer and «Done», no second
/// «Shield again» (BUG-274).
///
/// r1 Should 2, a known limit: it shares the pool's [`PoolState`] (its flow, progress and
/// answer) with «Shield», and its answer is `DoneWhat::Shield` as theirs — the core has no
/// answer of its own for it. One pool move runs at a time, so the two never wait at once; but
/// «Shield» opened right after a refund shows the refund's answer until a new amount is typed.
/// A `PoolMove::Refund` of its own would need the answer told apart by what was signed, not by
/// `DoneWhat`.
fn refund_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut CreateIdentityForm,
    shield: &mut PoolState,
    back: Duffs,
) {
    let busy = shield.flow.waiting();
    let ask = crate::components::password_asked(cx.snap) && !busy;
    let answer = match &shield.result {
        Some((PoolMove::Shield, result)) if !shield.flow.password.is_wrong() => {
            Some(result.clone())
        }
        _ => None,
    };
    let done = matches!(answer, Some(Ok(())));
    let mut send = false;
    let closed = crate::components::dialog_with_summary(
        ui,
        a,
        "shield-refund",
        "shield",
        tr!("Shield again"),
        440.0,
        |ui, a, part| {
            let p = a.p;
            match part {
                Part::Summary => {
                    if !done {
                        kv_row(
                            ui,
                            &p,
                            tr!("Back into the pool"),
                            &format!("≈ {}", dash(back)),
                            mono(13.0, Weight::Medium),
                        );
                    }
                    false
                }
                Part::Footer => {
                    if done {
                        let mut close = false;
                        button_row(ui, |ui| {
                            close = button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked();
                        });
                        return close;
                    }
                    let entered = shield
                        .flow
                        .password
                        .field(ui, &p, "shield-refund-password", ask);
                    let mut close = false;
                    button_row_spend(ui, |ui| {
                        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                            close = true;
                        }
                        let ready = !busy && shield.flow.password.ready(ask);
                        let label = if busy {
                            tr!("Shielding…")
                        } else {
                            tr!("Shield again")
                        };
                        if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked()
                            || (entered && ready)
                        {
                            send = true;
                        }
                    });
                    close
                }
                Part::Middle => {
                    match &answer {
                        Some(Ok(())) => notice(
                            ui,
                            a,
                            "confirmed",
                            p.accent,
                            tr!("The money is back in the pool."),
                        ),
                        Some(Err(e)) => error_notice(
                            ui,
                            a,
                            &crate::text::failed(tr!("Not shielded"), e),
                            crate::text::error_details(e).as_deref(),
                        ),
                        None => {
                            ui.add(crate::components::para(
                                tr!(
                                    "What Platform returned goes back into the pool, less the fee. Nothing moves on Core."
                                ),
                                13.0,
                                Weight::Regular,
                                p.muted,
                            ));
                            if let Some(fraction) = shield.progress {
                                crate::screens::shielded::proof_progress(ui, a, fraction);
                            }
                        }
                    }
                    false
                }
            }
        },
    );
    if send
        && let Some(command) = shield
            .flow
            .command(Spend::ShieldRefund, ask, PoolMove::Shield)
    {
        cx.send(command);
        shield.progress = Some(0.0);
        shield.result = None;
    }
    if closed && !busy {
        form.pool.refund = None;
        shield.result = None;
        shield.progress = None;
        shield.flow.password.forget();
    }
}

/// FEAT-0047: an invitation is a way of its own, and not a private one — said before anything.
fn invitation(ui: &mut Ui, a: &mut Assets, form: &mut CreateIdentityForm) {
    let p = a.p;
    let width = ui.available_width();
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 14.0;
        ui.label(super::txt(
            tr!("I have an invitation"),
            15.0,
            Weight::SemiBold,
            p.text,
        ));
        notice(
            ui,
            a,
            "warning",
            p.danger,
            tr!(
                "An identity from an invitation is not private: the person who invited you paid for it from their Core coins and knows which identity it is."
            ),
        );
        notice(
            ui,
            a,
            "info",
            p.muted,
            tr!("Accepting invitations comes in a later version."),
        );
        if button(ui, a, tr!("Back"), ButtonKind::Secondary, None).clicked() {
            form.pool.invitation = false;
        }
    });
}

/// What «Create privately» signs.
struct Plan {
    exit: Duffs,
    fee: Duffs,
    /// The name written in this run; `None`: none, or a contested one claimed after.
    label: Option<String>,
    claim: Option<String>,
    /// Step 2 (path M): from fully mixed coins, not the pool.
    mixed: bool,
}

/// The review: what leaves the pool, what the identity gets, what is private and what is seen,
/// and the password. «Sign and create» sends `CreateIdentityFromPool`.
#[allow(clippy::too_many_arguments)]
fn review(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut CreateIdentityForm,
    plan: &Plan,
    ask: bool,
    form_ready: bool,
    chosen: Option<String>,
) {
    let p = a.p;
    let mut create = false;
    let closed = crate::components::dialog_with_summary(
        ui,
        a,
        "create-identity-pool",
        "identity",
        tr!("Create privately"),
        440.0,
        |ui, a, part| {
            // BUG-249: what is agreed to, pinned over the password.
            if part == Part::Summary && plan.mixed {
                ui.spacing_mut().item_spacing.y = 14.0;
                mixed_lock::plan_lines(ui, a, &mut form.pool.mixed, MixedTo::Identity, plan.exit);
                form.flow.password.summary_tick(ui, &p);
                return false;
            }
            if part == Part::Summary {
                ui.spacing_mut().item_spacing.y = 14.0;
                let value = mono(13.0, Weight::Medium);
                kv_row(
                    ui,
                    &p,
                    tr!("From the pool"),
                    &dash(plan.exit),
                    value.clone(),
                );
                // BUG-319: promised the safe way — the fee at most, the credits at least.
                kv_row(
                    ui,
                    &p,
                    tr!("Platform fee"),
                    &format!(tr!("up to ≈ {}"), dash(plan.fee)),
                    value.clone(),
                );
                kv_row(
                    ui,
                    &p,
                    tr!("Into credits"),
                    &format!(
                        tr!("at least ≈ {}"),
                        dash(plan.exit.saturating_sub(plan.fee))
                    ),
                    value,
                );
                ui.add(crate::components::para(
                    tr!("The exact fee depends on the network's load; what came in is in History."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
                form.flow.password.summary_tick(ui, &p);
                return false;
            }
            if part == Part::Footer {
                form.flow
                    .password
                    .field(ui, &p, "create-identity-pool-password", ask);
                let mut close = false;
                crate::components::button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready = form_ready
                        && form.flow.password.ready(ask)
                        && (!plan.mixed || form.pool.mixed.ready(MixedTo::Identity, plan.exit));
                    if button_enabled(
                        ui,
                        a,
                        tr!("Sign and create"),
                        ButtonKind::Primary,
                        None,
                        ready,
                    )
                    .clicked()
                        && ready
                    {
                        create = true;
                        close = true;
                    }
                });
                return close;
            }
            ui.spacing_mut().item_spacing.y = 14.0;
            if plan.mixed {
                mixed_private_lines(ui, a);
            } else {
                private_lines(ui, a, plan.exit);
            }
            let public = match (&plan.label, &plan.claim) {
                (Some(label), _) => format!(
                    tr!("The name {} will be visible to everyone: people find you by it and can add you to contacts."),
                    crate::i18n::own(&format!("@{label}"))
                ),
                (None, Some(label)) => format!(
                    tr!(
                        "@{label} is not part of this: masternodes award it by vote. Right after, Identity offers its claim — the fund {fund}, not returned, and a password of its own."
                    ),
                    label = label,
                    fund = dash(cx.wallet().fees.name_contest_fund)
                ),
                (None, None) => tr!("Without a name nobody finds it by @name or adds it to DashPay contacts. You can add a name later on the Identity screen.").into(),
            };
            notice(ui, a, "eye", p.muted, &public);
            false
        },
    );
    if create {
        let op = if plan.mixed {
            Spend::CreateIdentityFromMixed {
                name: plan.label.clone(),
                amount: plan.exit,
                accept_excess: form.pool.mixed.accept_excess(MixedTo::Identity, plan.exit),
            }
        } else {
            Spend::CreateIdentityFromPool {
                name: plan.label.clone(),
                exit: plan.exit,
            }
        };
        let command = form.flow.command(op, ask, ());
        form.pool.progress = None;
        form.pool.sent = true;
        form.pool.sent_mixed = plan.mixed;
        form.sent(command, chosen, cx);
        form.pool.confirming = false;
    } else if closed {
        form.pool.confirming = false;
        form.flow.password.forget();
    }
}
