//! App Connect v2 on screen (FEAT-0023 round 4, docs/design/app-connect-v2-2026-11-11.md):
//! the request (A1, A1b, renewal R2), the refusals (E1–E3), how renewing works (R1) and
//! removing a disconnected app from the list (D2).
//!
//! One spending limit and one period for the whole connection: the keys share the limit, so
//! the optional ones get what they ask, at most an equal share, and the primary one the rest
//! ([`split`]). The person can only lower what the app asks; the core checks the same limits.

use super::app_requests::{AppRequests, Consent, Pick};
use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, attention_color, button, button_enabled, checkbox, dialog,
    kv_row, notice, segmented, text_field,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::{Weight, mono};
use chrono::{DateTime, Local};
use egui::{Frame, Margin, Stroke, Ui};
use evolite_api::{
    AppBinding, AppLinkRefusal, AppRequest, AppRequestKind, Duffs, EncryptionOffer, Renewal,
    SessionGrant, SessionLimits, SessionOffer, Timestamp,
};

const DAY: Timestamp = 24 * 60 * 60;

/// What the person chose in the request dialog: the limit (a preset or their amount), the
/// period (a preset or their date) and which optional keys stay.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
    /// Of [`SessionLimits::budget_presets`], or past them: «Your amount».
    pub budget: usize,
    pub amount: String,
    /// Of [`SessionLimits::term_presets_days`], or past them: «Your date».
    pub term: usize,
    pub date: String,
    /// Per asked key; the primary is always kept.
    pub kept: Vec<bool>,
    /// The request these choices are for.
    pub for_request: u64,
}

impl Choice {
    /// What is selected when a request opens: what the app asks for (within the wallet's limits,
    /// as the core offered it), else the wallet's defaults — 5 DASH and 90 days.
    pub fn of(request: &AppRequest, now: Timestamp) -> Self {
        let AppRequestKind::Connect2 {
            sessions, limits, ..
        } = &request.kind;
        let primary = sessions.first();
        // The connection's one limit: what the app asks for all its keys, at most 5 DASH (the
        // architect, round 4; the core offered each within the wallet's limits).
        let budget = match sessions.iter().map(|s| s.budget).sum::<Duffs>() {
            0 => SessionLimits::DEFAULT_BUDGET,
            asked => asked.min(limits.max_budget),
        };
        let end = primary.map_or(now + SessionLimits::DEFAULT_TERM_DAYS * DAY, |s| {
            s.expires_at
        });
        let (budget, amount) = match limits.budget_presets.iter().position(|&p| p == budget) {
            Some(i) => (i, String::new()),
            None => (
                limits.budget_presets.len(),
                crate::format::input_text(budget),
            ),
        };
        // A period of whole days the app asks is one of the presets when it is within a day.
        let term = limits
            .term_presets_days
            .iter()
            .position(|&d| (now + d * DAY - end).abs() < DAY);
        let (term, date) = match term {
            Some(i) => (i, String::new()),
            None => (limits.term_presets_days.len(), text::date_input(end)),
        };
        Self {
            budget,
            amount,
            term,
            date,
            kept: vec![true; sessions.len()],
            for_request: request.id,
        }
    }

    /// The limit chosen, if it is one the wallet takes.
    pub fn limit(&self, limits: &SessionLimits) -> Option<Duffs> {
        let amount = match limits.budget_presets.get(self.budget) {
            Some(&preset) => preset,
            None => crate::format::parse_amount(&self.amount)?,
        };
        (1..=limits.max_budget).contains(&amount).then_some(amount)
    }

    /// The end chosen (Unix seconds), if it is one the wallet takes: an hour to a year away.
    pub fn end(&self, limits: &SessionLimits, now: Timestamp) -> Option<Timestamp> {
        let end = match limits.term_presets_days.get(self.term) {
            Some(&days) => now + days * DAY,
            None => text::date_from_input(&self.date)?,
        };
        (now + limits.min_term..=now + limits.max_term)
            .contains(&end)
            .then_some(end)
    }
}

/// The kept optional keys' budgets out of `limit` (the architect, round 4): what each asks,
/// while that leaves the primary key something; else all scaled down in proportion to half of
/// `limit`. `None` when even that leaves a key or the primary one with nothing.
pub fn optional_budgets(asks: &[Duffs], limit: Duffs) -> Option<Vec<Duffs>> {
    let asked: Duffs = asks.iter().sum();
    if asked < limit {
        return Some(asks.to_vec());
    }
    let room = limit / 2;
    let scaled: Vec<Duffs> = asks
        .iter()
        .map(|&ask| (ask as i128 * room as i128 / asked.max(1) as i128) as Duffs)
        .collect();
    let given: Duffs = scaled.iter().sum();
    (scaled.iter().all(|&b| b > 0) && limit - given > 0).then_some(scaled)
}

/// The grant for the core: the kept keys share `limit` — the optional ones as
/// [`optional_budgets`] gives them, the primary one the rest — and all end at `end`. Together
/// never more than `limit`, the primary key never nothing, an optional one never more than it
/// asks. `None`: the limit is less than the app needs.
pub fn split(
    sessions: &[SessionOffer],
    kept: &[bool],
    limit: Duffs,
    end: Timestamp,
) -> Option<SessionGrant> {
    let optional: Vec<usize> = (0..sessions.len())
        .filter(|&i| kept[i] && !sessions[i].primary)
        .collect();
    let asks: Vec<Duffs> = optional.iter().map(|&i| sessions[i].budget).collect();
    let budgets = optional_budgets(&asks, limit)?;
    let mut keys: Vec<Option<(Duffs, Timestamp)>> = vec![None; sessions.len()];
    for (&i, &budget) in optional.iter().zip(&budgets) {
        keys[i] = Some((budget, end));
    }
    let primary = sessions.iter().position(|s| s.primary)?;
    let left = limit - budgets.iter().sum::<Duffs>();
    (left > 0).then_some(())?;
    keys[primary] = Some((left, end));
    Some(SessionGrant { keys })
}

/// What an app may do with a key, in the person's words.
pub fn binding_words(binding: &AppBinding, app: &str) -> String {
    match binding {
        AppBinding::Group { .. } | AppBinding::Contract { .. } => {
            format!(tr!("Write to the {} contracts in your name"), app)
        }
        AppBinding::DashPayProfile => tr!("Change your DashPay profile").to_string(),
        AppBinding::DocumentType { name, .. } => {
            format!(tr!("Write {} documents in your name"), name)
        }
    }
}

/// The same, shorter, for the list.
pub fn binding_title(binding: &AppBinding, app: &str) -> String {
    match binding {
        AppBinding::Group { .. } | AppBinding::Contract { .. } => {
            format!(tr!("Write to the {} contracts"), app)
        }
        AppBinding::DashPayProfile => tr!("Change your DashPay profile").to_string(),
        AppBinding::DocumentType { name, .. } => format!(tr!("Write {} documents"), name),
    }
}

/// "a minute ago"-style age of a request.
fn age(seconds: Timestamp) -> String {
    let minutes = (seconds / 60).max(1) as u32;
    text::plural(minutes, tr!("minute"), tr!("minutes"))
}

/// A limit with its unit, without trailing zeros (0.5 DASH, as the mockups write it).
fn dash(d: Duffs) -> String {
    format!(
        "{} {}",
        crate::format::short_number(d),
        crate::format::unit_name()
    )
}

/// The number alone, for the presets.
fn number(d: Duffs) -> String {
    crate::format::short_number(d)
}

/// The explanation under the limit and period (A1b, point 9).
/// `sessions`: all the keys asked; the app's amount is what it asks for all of them together
/// (the one limit, the architect, round 4), its period the primary key's.
fn explanation(sessions: &[SessionOffer], now: DateTime<Local>) -> String {
    let rules = tr!(
        "evolite allows at most 5 DASH, and a period from an hour to a year. After that the limit changes only with a new sign-in."
    );
    let Some(primary) = sessions.first() else {
        return String::new();
    };
    let proposed: Duffs = sessions.iter().filter_map(|s| s.proposed_budget).sum();
    let total = (proposed > 0).then_some(proposed);
    let asked = match (total, primary.proposed_expiry) {
        (Some(amount), Some(until)) => format!(
            tr!(
                "The app asks for {amount} and {days} — until {date}. That is what is selected; you can choose less."
            ),
            amount = dash(amount),
            days = text::plural(
                // Rounded: the request was made a moment before it is shown.
                ((until - now.timestamp() + DAY / 2) / DAY).max(1) as u32,
                tr!("day"),
                tr!("days")
            ),
            date = text::date_long(until),
        ),
        (None, _) => {
            tr!("The app did not name an amount; evolite sets 5 DASH. You can choose less.")
                .to_string()
        }
        (Some(amount), None) => format!(
            tr!("The app asks for {}. That is what is selected; you can choose less."),
            dash(amount)
        ),
    };
    let allowed: Duffs = sessions
        .iter()
        .map(|s| s.budget)
        .sum::<Duffs>()
        .min(SessionLimits::MAX_BUDGET);
    let lowered = match total {
        Some(asked) if asked > allowed => {
            format!(
                tr!("The app asks for {asked}; evolite lowered it to {allowed}."),
                asked = dash(asked),
                allowed = dash(allowed)
            ) + " "
        }
        _ => String::new(),
    };
    format!(
        "{lowered}{} {}",
        crate::i18n::t(&asked),
        crate::i18n::t(rules)
    )
}

fn wrapped(ui: &mut Ui, text: &str, size: f32, color: egui::Color32) {
    ui.add(
        egui::Label::new(txt(text, size, Weight::Regular, color).line_height(Some(size * 1.5)))
            .wrap(),
    );
}

fn section(ui: &mut Ui, a: &Assets, text: &str) {
    ui.label(txt(text, 13.0, Weight::SemiBold, a.p.muted));
}

/// A line of what the app will be able to do, or not: a tick, a checkbox or a cross.
fn can_line(
    ui: &mut Ui,
    a: &mut Assets,
    icon: &str,
    title: &str,
    under: Option<&str>,
    muted: bool,
) {
    let p = a.p;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.add(
            a.icons
                .icon(icon, if muted { p.muted } else { p.text }, &p, 18.0),
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(txt(
                title,
                14.0,
                Weight::Regular,
                if muted { p.muted } else { p.text },
            ));
            if let Some(under) = under {
                wrapped(ui, under, 12.0, p.muted);
            }
        });
    });
}

/// The «attention» frame of the warning about older apps (R-6): accent border, warning icon.
fn permanent(ui: &mut Ui, a: &mut Assets, identity: &str) {
    let p = a.p;
    Frame::new()
        .stroke(Stroke::new(1.0, attention_color(&p)))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(a.icons.icon("warning", attention_color(&p), &p, 18.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(txt(
                        &format!(tr!("This is permanent for {}"), identity),
                        14.0,
                        Weight::SemiBold,
                        p.text,
                    ));
                    wrapped(
                        ui,
                        tr!("After you connect, this identity will no longer open in older apps: Yappr on yap.pr (except /staging) and DashPay 11.9.1 and earlier. This cannot be undone, even if you disconnect the app later."),
                        13.0,
                        p.text,
                    );
                });
            });
        });
}

/// The renewal note in place of the warning (R2).
fn renewal_note(ui: &mut Ui, a: &mut Assets, app: &str, old: &Renewal) {
    let p = a.p;
    let spent = old.spent.map_or_else(|| "—".to_owned(), dash);
    notice(
        ui,
        a,
        "info",
        p.muted,
        &format!(
            "{}\n{}",
            crate::i18n::t(tr!("This is a renewal")),
            crate::i18n::t(&format!(
                tr!(
                    "The old {} keys (spent {} of {}, until {}) will be disabled in the same identity update."
                ),
                app,
                spent,
                dash(old.budget),
                text::date_long(old.until)
            ))
        ),
    );
}

/// The request dialog (A1, A1b; R2 for a renewal). Returns what the person picked.
pub fn request(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut AppRequests,
    request: &AppRequest,
) -> Option<Pick> {
    let AppRequestKind::Connect2 {
        app_id,
        label,
        owner,
        owner_name,
        identity,
        sessions,
        encryption,
        limits,
        breaks_old_clients,
        renews,
        created_at,
        cost,
        ..
    } = &request.kind;
    let now = cx.now;
    if state.choice.for_request != request.id {
        state.choice = Choice::of(request, now.timestamp());
    }
    let app = label.as_str();
    let title = match renews {
        Some(_) => format!(tr!("Renew the {} connection"), app),
        None => tr!("Connect an app").to_string(),
    };
    let mut pick = None;
    let closed = dialog(
        ui,
        a,
        "app-connect-v2",
        "app-connect",
        &title,
        560.0,
        |ui, a| {
            let p = a.p;
            ui.spacing_mut().item_spacing.y = 12.0;
            // Everything between the title and the password scrolls (BUG-90).
            let room = (ui.ctx().content_rect().height() - 330.0).max(140.0);
            let mut body = egui::ScrollArea::vertical()
                .id_salt("app-connect-v2-body")
                .auto_shrink([false, true])
                .max_height(room);
            if state.body_to_end {
                body = body.stick_to_bottom(true);
            }
            body.show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.horizontal(|ui| {
                    crate::components::network_chip(ui, a, cx.snap.network);
                    ui.label(txt(
                        &format!(
                            tr!("App Connect v2 · request made {} ago"),
                            age(now.timestamp() - created_at)
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                });
                // The app: its own words, and who owns its contract (R-8).
                Frame::new()
                    .stroke(Stroke::new(1.0, p.line))
                    .corner_radius(12)
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_top(|ui| {
                            let (logo, _) =
                                ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                            crate::components::paint_letter_box(ui, &p, logo, app);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.label(txt(app, 15.0, Weight::SemiBold, p.text));
                                ui.label(txt(
                                    tr!("what the app calls itself"),
                                    12.0,
                                    Weight::Regular,
                                    p.muted,
                                ));
                                let owner = owner_name
                                    .as_ref()
                                    .map_or_else(|| text::short_address(owner), |n| format!("@{n}"));
                                ui.label(txt(
                                    &format!(
                                        tr!("contract {} · owner {}"),
                                        text::short_address(app_id),
                                        owner
                                    ),
                                    12.0,
                                    Weight::Regular,
                                    p.muted,
                                ));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                crate::components::chip(
                                    ui,
                                    a,
                                    "confirmed",
                                    tr!("Contract found"),
                                    false,
                                );
                            });
                        });
                    });
                // BUG-29 (c): a row of text, the identity of «Work as»; with more than one, where
                // to change it.
                section(ui, a, tr!("Sign in as"));
                ui.label(txt(identity, 14.0, Weight::Medium, p.text));
                if cx.wallet().identities.len() > 1 {
                    ui.label(txt(
                        tr!("Signs in with the identity chosen in “Work as”; you can change it there."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
                section(ui, a, tr!("The app will be able to"));
                let limit = state.choice.limit(limits).unwrap_or(0);
                // What each optional key would get now: its ask, or less when the limit is small.
                let shares = split(sessions, &state.choice.kept, limit, 0);
                for (i, offer) in sessions.iter().enumerate() {
                    if offer.primary {
                        can_line(
                            ui,
                            a,
                            "check",
                            &binding_words(&offer.binding, app),
                            Some(&format!(
                                tr!("fees come from the {} credits, within the limit below"),
                                identity
                            )),
                            false,
                        );
                    } else {
                        let mut on = state.choice.kept[i];
                        checkbox(ui, a, &mut on, &binding_words(&offer.binding, app));
                        state.choice.kept[i] = on;
                        let up_to = shares
                            .as_ref()
                            .and_then(|g| g.keys[i].map(|(b, _)| b))
                            .unwrap_or(offer.budget);
                        ui.horizontal(|ui| {
                            ui.add_space(32.0);
                            ui.label(txt(
                                &format!(
                                    tr!("up to {} from the same limit · you can leave it off"),
                                    dash(up_to)
                                ),
                                12.0,
                                Weight::Regular,
                                p.muted,
                            ));
                        });
                    }
                }
                can_line(ui, a, "close", tr!("Transfer credits and DASH"), None, true);
                can_line(ui, a, "close", tr!("See the phrase and other keys"), None, true);
                for enc in encryption.iter().filter(|e| e.added) {
                    let EncryptionOffer { contract, .. } = enc;
                    let whose = if contract == app_id { app.to_owned() } else { text::short_address(contract) };
                    wrapped(
                        ui,
                        &format!(
                            tr!("evolite also adds an encryption key for {} messages; it spends nothing."),
                            whose
                        ),
                        12.0,
                        p.muted,
                    );
                }
                crate::components::divider(ui, &p);
                section(ui, a, tr!("Spending limit, DASH"));
                let mut items: Vec<String> = limits.budget_presets.iter().map(|&d| number(d)).collect();
                items.push(crate::i18n::t(tr!("Your amount")).into_owned());
                let refs: Vec<&str> = items.iter().map(String::as_str).collect();
                segmented(ui, &p, &refs, &mut state.choice.budget, false);
                if state.choice.budget >= limits.budget_presets.len() {
                    crate::a11y::name_next_field(ui, tr!("Your amount"));
                    text_field(
                        ui,
                        &p,
                        "app-connect-amount",
                        &mut state.choice.amount,
                        "0.25",
                        FieldKind::Amount,
                        |_| {},
                    );
                }
                section(ui, a, tr!("Period"));
                let mut items: Vec<String> = limits
                    .term_presets_days
                    .iter()
                    .map(|&d| match d {
                        365 => crate::i18n::t(tr!("1 year")).into_owned(),
                        d => text::plural(d as u32, tr!("day"), tr!("days")),
                    })
                    .collect();
                items.push(crate::i18n::t(tr!("Your date")).into_owned());
                let refs: Vec<&str> = items.iter().map(String::as_str).collect();
                segmented(ui, &p, &refs, &mut state.choice.term, false);
                if state.choice.term >= limits.term_presets_days.len() {
                    crate::a11y::name_next_field(ui, tr!("Your date"));
                    text_field(
                        ui,
                        &p,
                        "app-connect-date",
                        &mut state.choice.date,
                        tr!("dd.mm.yyyy"),
                        FieldKind::Text,
                        |_| {},
                    );
                }
                wrapped(ui, &explanation(sessions, now), 12.0, p.muted);
                match renews {
                    Some(old) => renewal_note(ui, a, app, old),
                    None if *breaks_old_clients => permanent(ui, a, identity),
                    None => {}
                }
                kv_row(
                    ui,
                    &p,
                    if renews.is_some() {
                        tr!("New keys and disabling the old ones")
                    } else {
                        tr!("Adding keys")
                    },
                    &format!("≈ {}", dash(*cost)),
                    mono(13.0, Weight::Medium),
                );
                kv_row(
                    ui,
                    &p,
                    tr!("Pays"),
                    &format!(tr!("{} · credits"), identity),
                    mono(13.0, Weight::Medium),
                );
            });
            // Pinned: the password and the buttons (asked always: this cannot be undone). The
            // app asks, not the person: Enter in the field never connects (rule b′, BUG-30).
            if state.consent == Consent::Asking {
                state.password.field(ui, &p, "app-request-password", true);
            }
            let end = state.choice.end(limits, now.timestamp());
            let limit = state.choice.limit(limits);
            let fits = limit.is_some_and(|l| split(sessions, &state.choice.kept, l, 0).is_some());
            let ready = state.password.ready(true) && end.is_some() && limit.is_some() && fits;
            let yes = if renews.is_some() {
                tr!("Renew")
            } else {
                tr!("Connect")
            };
            crate::components::button_row_destructive_if(ui, true, |ui| match state.consent {
                Consent::Asking => {
                    if button(ui, a, tr!("Decline"), ButtonKind::Ghost, None).clicked() {
                        pick = Some(Pick::No);
                    }
                    if button_enabled(ui, a, yes, ButtonKind::Primary, None, ready).clicked() {
                        pick = Some(Pick::Yes);
                    }
                }
                Consent::Sending => {
                    button_enabled(ui, a, tr!("Decline"), ButtonKind::Ghost, None, false);
                    button_enabled(ui, a, tr!("Sending…"), ButtonKind::Primary, None, false);
                }
                Consent::Sent | Consent::Failed => {
                    if button(ui, a, tr!("Close"), ButtonKind::Primary, None).clicked() {
                        pick = Some(Pick::Close);
                    }
                }
            });
            if state.consent == Consent::Asking {
                let why = if !state.password.ready(true) {
                    Some(tr!("Enter the password"))
                } else if limit.is_none() {
                    Some(tr!("Choose a limit of at most 5 DASH"))
                } else if end.is_none() {
                    Some(tr!("Choose a date between an hour and a year from now"))
                } else if !fits {
                    Some(tr!("The limit is less than the app needs"))
                } else {
                    None
                };
                if let Some(why) = why {
                    ui.label(txt(why, 12.0, Weight::Regular, p.muted));
                }
            }
            if let Some(Err(e)) = &state.result
                && !state.password.is_wrong()
            {
                crate::components::error_notice(
                    ui,
                    a,
                    &text::failed(tr!("Not connected"), e),
                    text::error_details(e).as_deref(),
                );
            }
            false
        },
    );
    if closed && pick.is_none() {
        pick = Some(Pick::Close);
    }
    pick
}

/// The grant the dialog's choices make, if they are ones the wallet takes.
pub fn grant(request: &AppRequest, choice: &Choice, now: Timestamp) -> Option<SessionGrant> {
    let AppRequestKind::Connect2 {
        sessions, limits, ..
    } = &request.kind;
    let limit = choice.limit(limits)?;
    let end = choice.end(limits, now)?;
    split(sessions, &choice.kept, limit, end)
}

/// A refused link in words (E1–E3 and the rest): title, what happened, what to do. Nothing was
/// spent.
pub fn refusal_words(why: &AppLinkRefusal, app: &str) -> (String, String, String) {
    let nothing = tr!("Nothing was spent.");
    let t = |s: &str| crate::i18n::t(s).into_owned();
    match why {
        AppLinkRefusal::Version { v } => (
            t(tr!("Cannot connect")),
            format!(
                tr!(
                    "evolite works only with App Connect v2, and this link is another version (v={})."
                ),
                v
            ),
            t(tr!(
                "Ask the app for an App Connect v2 link. Nothing was spent."
            )),
        ),
        AppLinkRefusal::Stale => (
            t(tr!("The request is out of date")),
            t(tr!(
                "The link is more than 10 minutes old: the app no longer waits for an answer to it."
            )),
            format!(
                tr!("Refresh the {} page and paste the new link. Nothing was spent."),
                app
            ),
        ),
        AppLinkRefusal::Mainnet => (
            t(tr!("Test network only")),
            t(tr!(
                "App Connect v2 works only on the test network and devnet for now. This request is for the main network."
            )),
            t(tr!(
                "Connecting on the main network will come when the protocol is ready there. Nothing was spent."
            )),
        ),
        other => {
            let reason = match other {
                AppLinkRefusal::OtherNetwork { n } => {
                    format!(tr!("The request is for another network ({})."), n)
                }
                AppLinkRefusal::SystemContract => {
                    t(tr!("The app asks for access to a system contract."))
                }
                AppLinkRefusal::OthersContract => t(tr!(
                    "The app asks for a contract or group that belongs to someone else."
                )),
                AppLinkRefusal::UnknownField => {
                    t(tr!("The request has a field evolite does not know."))
                }
                AppLinkRefusal::NoContract => t(tr!("The app's contract is not on the network.")),
                AppLinkRefusal::NoEncryptionKeys => t(tr!(
                    "The app asks for an encryption key of a contract that keeps none."
                )),
                AppLinkRefusal::OldNetwork => t(tr!(
                    "The network does not take keys with a limit and a period yet."
                )),
                _ => t(tr!("The request is not well formed.")),
            };
            (t(tr!("Cannot connect")), reason, t(nothing))
        }
    }
}

/// The refusal dialog (E1–E3): «attention» tone, never red. Returns true when closed; with
/// `again` the person asks to paste another link.
pub fn refusal(ui: &mut Ui, a: &mut Assets, why: &AppLinkRefusal, again: &mut bool) -> bool {
    let (title, body, hint) = refusal_words(why, tr!("app"));
    dialog(
        ui,
        a,
        "app-link-refused",
        "warning",
        &title,
        470.0,
        |ui, a| {
            let p = a.p;
            ui.spacing_mut().item_spacing.y = 14.0;
            wrapped(ui, &body, 14.0, p.text);
            notice(ui, a, "info", p.muted, &hint);
            let mut close = false;
            crate::components::button_row(ui, |ui| {
                if matches!(why, AppLinkRefusal::Stale)
                    && button(ui, a, tr!("Paste another"), ButtonKind::Ghost, None).clicked()
                {
                    *again = true;
                    close = true;
                }
                if button(ui, a, tr!("Got it"), ButtonKind::Primary, None).clicked() {
                    close = true;
                }
            });
            close
        },
    )
}

/// «Renew» (R1): the limit cannot be raised in place; a new sign-in with the same connection
/// renews it. Returns what was clicked: `Some(true)` paste, `Some(false)` scan.
pub fn renew_how(
    ui: &mut Ui,
    a: &mut Assets,
    app: &str,
    now_line: &str,
    closed: &mut bool,
) -> Option<bool> {
    let mut pick = None;
    *closed = dialog(
        ui,
        a,
        "app-renew",
        "import",
        &format!(tr!("Renew the {} connection"), app),
        470.0,
        |ui, a| {
            let p = a.p;
            ui.spacing_mut().item_spacing.y = 12.0;
            wrapped(
                ui,
                &format!(
                    tr!(
                        "The limit and period cannot simply be raised: it takes a new sign-in. Open {app}, sign in again and paste or scan its link."
                    ),
                    app = app
                ),
                14.0,
                p.text,
            );
            wrapped(
                ui,
                tr!(
                    "evolite recognises this connection, issues new keys with the new limit and period, and disables the old ones in the same identity update."
                ),
                14.0,
                p.text,
            );
            kv_row(ui, &p, tr!("Now"), now_line, mono(13.0, Weight::Medium));
            let mut close = false;
            crate::components::button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                if button(ui, a, tr!("Scan QR"), ButtonKind::Secondary, None).clicked() {
                    pick = Some(false);
                    close = true;
                }
                if button(ui, a, tr!("Paste a link"), ButtonKind::Primary, None).clicked() {
                    pick = Some(true);
                    close = true;
                }
            });
            close
        },
    );
    pick
}

/// «Remove from the list» (D2): only the record on this computer; no password, nothing spent.
/// Returns true to remove; `closed` when the dialog goes.
pub fn remove_dialog(ui: &mut Ui, a: &mut Assets, app: &str, closed: &mut bool) -> bool {
    let mut remove = false;
    *closed = dialog(
        ui,
        a,
        "app-remove",
        "close",
        tr!("Remove from the list?"),
        470.0,
        |ui, a| {
            let p = a.p;
            ui.spacing_mut().item_spacing.y = 12.0;
            wrapped(
                ui,
                &format!(
                    tr!(
                        "{} is already disconnected. Only the record on this computer leaves the list."
                    ),
                    app
                ),
                14.0,
                p.text,
            );
            notice(
                ui,
                a,
                "info",
                p.muted,
                tr!(
                    "Disabled keys stay on the identity forever: that is how the network works. Nothing is spent, no password is needed."
                ),
            );
            let mut close = false;
            crate::components::button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                if button(
                    ui,
                    a,
                    tr!("Remove from the list"),
                    ButtonKind::Primary,
                    None,
                )
                .clicked()
                {
                    remove = true;
                    close = true;
                }
            });
            close
        },
    );
    remove
}

/// «… connected: limit … until …» over the list once a sign-in is done (A1, after).
pub fn connected_banner(request: &AppRequest, grant: &SessionGrant) -> String {
    let total: Duffs = grant.keys.iter().flatten().map(|(b, _)| b).sum();
    let until = grant
        .keys
        .iter()
        .flatten()
        .map(|(_, e)| *e)
        .max()
        .unwrap_or(0);
    let app = match &request.kind {
        AppRequestKind::Connect2 { label, .. } => label.clone(),
    };
    format!(
        tr!("{} connected: limit {} until {}"),
        app,
        dash(total),
        text::date_long(until)
    )
}

#[cfg(test)]
mod tests;
