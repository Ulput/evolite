//! 10 · Apps (FEAT-0023 round 4, L1/L2): the apps signed in as the identity by App Connect v2,
//! each with what it may do, how much of its limit is spent and until when; «Renew» and
//! «Disconnect»; the disconnected ones below with «Remove from the list». How much is left
//! comes from Platform (R-3): asked once a visit.

use super::app_requests::{self, AppRequests, app_label};
use super::connect2::{self, binding_title};
use super::txt;
use crate::components::{
    Assets, ButtonKind, attention_color, button, button_enabled, card, inner, notice,
    notice_closable, page_header, paint_letter_box,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::Weight;
use egui::{CornerRadius, Margin, Sense, Stroke, Ui, vec2};
use evolite_api::{ConnectedApp, SessionKey, SessionKeyState, Timestamp};

/// An example link, so the request can be seen without a real app (the demo).
#[cfg(demo_core)]
const EXAMPLE: &str = "dashpay://connect?v=2&n=t&r=AQ";

/// Where a connection stands, by its keys (the design's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppState {
    Active,
    /// A key's budget is used up: the app can write nothing more.
    UsedUp,
    /// The primary key's end has passed.
    Expired(Timestamp),
    Disconnected,
}

pub fn state_of(app: &ConnectedApp) -> AppState {
    if app.disconnected_at.is_some() {
        return AppState::Disconnected;
    }
    if app.session.iter().any(|k| k.remaining == Some(0)) {
        return AppState::UsedUp;
    }
    match app.session.first() {
        Some(k) if k.state == SessionKeyState::Expired => AppState::Expired(k.expires_at),
        _ => AppState::Active,
    }
}

/// «Spent X of Y · until Z» of a key; «—» for X until Platform was asked.
fn spent_line(key: &SessionKey, now: chrono::DateTime<chrono::Local>) -> String {
    let spent = key.remaining.map_or_else(
        || "—".to_owned(),
        |left| crate::format::short_number(key.budget - left),
    );
    format!(
        tr!("Spent {} of {} · until {}"),
        spent,
        format!(
            "{} {}",
            crate::format::short_number(key.budget),
            crate::format::unit_name()
        ),
        text::date_short(key.expires_at, now)
    )
}

/// The spent part of a key's budget, 0 until read.
fn spent_fraction(key: &SessionKey) -> f32 {
    match key.remaining {
        Some(left) if key.budget > 0 => (key.budget - left) as f32 / key.budget as f32,
        _ => 0.0,
    }
}

/// A 4 px bar in `color` (the design: text when active, accent when used up, muted after).
fn bar(ui: &mut Ui, track: egui::Color32, color: egui::Color32, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), track);
    let mut done = rect;
    done.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    ui.painter().rect_filled(done, CornerRadius::same(2), color);
}

/// A state chip: quiet for «Active» and «Disconnected», an accent border otherwise; never red.
fn state_chip(ui: &mut Ui, a: &mut Assets, state: AppState, now: chrono::DateTime<chrono::Local>) {
    let p = a.p;
    let (icon, words, accent) = match state {
        AppState::Active => ("check", crate::i18n::t(tr!("Active")).into_owned(), false),
        AppState::UsedUp => (
            "warning",
            crate::i18n::t(tr!("Limit used up")).into_owned(),
            true,
        ),
        AppState::Expired(at) => (
            "calendar",
            format!(tr!("Expired on {}"), text::date_short(at, now)),
            true,
        ),
        AppState::Disconnected => (
            "close",
            crate::i18n::t(tr!("Disconnected")).into_owned(),
            false,
        ),
    };
    let color = if accent { attention_color(&p) } else { p.muted };
    egui::Frame::new()
        .fill(if accent { p.surface } else { p.raised })
        .stroke(Stroke::new(if accent { 1.0 } else { 0.0 }, color))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(a.icons.icon(icon, color, &p, 14.0));
                ui.label(txt(&words, 12.0, Weight::Medium, color));
            });
        });
}

/// What one card asks for.
#[derive(Default)]
struct CardAnswer {
    renew: bool,
    disconnect: bool,
    remove: bool,
}

/// One app's card (L1; L2 when disconnected).
fn app_card(
    ui: &mut Ui,
    a: &mut Assets,
    app: &ConnectedApp,
    who: &str,
    now: chrono::DateTime<chrono::Local>,
) -> CardAnswer {
    let p = a.p;
    let state = state_of(app);
    let label = if app.label.is_empty() {
        app_label(&app.name)
    } else {
        app.label.clone()
    };
    let mut answer = CardAnswer::default();
    let total = ui.available_width();
    card(&p, Margin::same(20)).show(ui, |ui| {
        ui.set_width(inner(total, 20));
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (logo, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            paint_letter_box(ui, &p, logo, &label);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(txt(&label, 15.0, Weight::SemiBold, p.text));
                let under = match app.disconnected_at {
                    Some(at) => format!(
                        tr!("disconnected on {} · the keys on the identity are disabled"),
                        text::date_short(at, now)
                    ),
                    None => {
                        let contract = format!(tr!("contract {}"), text::short_address(&app.id));
                        format!(
                            "{} · {} · {}",
                            crate::i18n::t(tr!("what the app calls itself")),
                            crate::i18n::t(&contract),
                            who
                        )
                    }
                };
                ui.label(txt(&under, 12.0, Weight::Regular, p.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                state_chip(ui, a, state, now);
            });
        });
        if state != AppState::Disconnected {
            for key in &app.session {
                ui.horizontal(|ui| {
                    ui.label(txt(
                        &binding_title(&key.binding, &label),
                        13.0,
                        Weight::Regular,
                        p.text,
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(txt(&spent_line(key, now), 12.0, Weight::Regular, p.muted));
                    });
                });
                let color = match state {
                    AppState::UsedUp => attention_color(&p),
                    AppState::Expired(_) => p.muted,
                    _ => p.text,
                };
                bar(ui, p.line, color, spent_fraction(key));
            }
            if state == AppState::UsedUp {
                notice(
                    ui,
                    a,
                    "info",
                    p.muted,
                    tr!("The app can no longer write anything in your name. Renew the connection to set a new limit."),
                );
            }
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            match state {
                AppState::Disconnected => {
                    if button(ui, a, tr!("Remove from the list"), ButtonKind::Ghost, None).clicked()
                    {
                        answer.remove = true;
                    }
                }
                _ => {
                    if button(ui, a, tr!("Renew"), ButtonKind::Secondary, None).clicked() {
                        answer.renew = true;
                    }
                    if button(ui, a, tr!("Disconnect"), ButtonKind::Ghost, None)
                        .on_hover_text(
                            crate::i18n::t(&format!(
                                tr!("{} will no longer be able to sign in as {who}"),
                                label,
                                who = who
                            ))
                            .into_owned(),
                        )
                        .clicked()
                    {
                        answer.disconnect = true;
                    }
                }
            }
        });
    });
    answer
}

pub fn apps(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, requests: &mut AppRequests) {
    let p = a.p;
    let wallet = cx.wallet();
    let name = wallet.identity.as_ref().and_then(|i| i.name());
    let who = name.map_or_else(|| "identity".to_string(), |n| format!("@{n}"));
    let now = cx.now;
    ui.spacing_mut().item_spacing.y = 22.0;
    // «Scan QR» (UC-4): an app's link goes where «Paste a link» sends it; a picture may also be
    // dropped on the screen.
    let screen = ui.max_rect();
    if !requests.pasting
        && requests.open.is_none()
        && let Some(picture) = crate::qr_scan::dropped(ui.ctx())
    {
        requests.scanner.start(ui.ctx(), picture);
    }
    if let Some(scanned) = requests.scanner.poll() {
        match app_link_from_qr(&scanned) {
            Ok(link) => {
                requests.qr_error = None;
                cx.send(evolite_api::Command::OpenAppLink { text: link });
            }
            Err(why) => requests.qr_error = why,
        }
    }
    // R-3: what is left comes from Platform, asked once a visit for each connected app.
    for app in wallet.apps.iter().filter(|a| a.disconnected_at.is_none()) {
        if requests.budgets_asked.insert(app.id.clone()) {
            cx.send(evolite_api::Command::ReadAppBudgets { id: app.id.clone() });
        }
    }
    let mut scan_now = false;
    page_header(ui, a, tr!("Apps"), None, |ui, a| {
        let scanning = requests.scanner.busy();
        let scan = button_enabled(ui, a, tr!("Scan QR"), ButtonKind::Primary, None, !scanning);
        crate::components::qr_menu_under(ui, a, "apps", &scan, &mut requests.scanner);
        if button(ui, a, tr!("Paste a link"), ButtonKind::Secondary, None).clicked() {
            requests.pasting = true;
            requests.qr_error = None;
        }
    });
    if let Some(why) = requests.qr_error {
        crate::components::qr_note(ui, a, &crate::components::QrNote::Error(why));
    }
    if !requests.pasting && requests.open.is_none() && crate::qr_scan::dragging_picture(ui.ctx()) {
        crate::components::drop_overlay(ui.ctx(), a, screen);
    }

    let connected: Vec<&ConnectedApp> = wallet
        .apps
        .iter()
        .filter(|a| a.disconnected_at.is_none())
        .collect();
    let disconnected: Vec<&ConnectedApp> = wallet
        .apps
        .iter()
        .filter(|a| a.disconnected_at.is_some())
        .collect();
    // `--screen apps-…` (dev-tools): a dialog of the list open, or the list at its end.
    let mut to_end = false;
    if !wallet.apps.is_empty()
        && let Some(preset) = requests.preset.take()
    {
        let name = |a: &ConnectedApp| a.label.clone();
        match (preset, connected.first(), disconnected.first()) {
            ("renew", Some(app), _) => {
                let line = app.session.first().map(|k| spent_line(k, now));
                requests.renewing = Some((name(app), line.unwrap_or_default()));
            }
            ("disconnect", Some(app), _) => {
                requests.disconnecting = Some((app.id.clone(), name(app)));
            }
            ("remove", _, Some(app)) => requests.removing = Some((app.id.clone(), name(app))),
            ("to-end", ..) => to_end = true,
            _ => {}
        }
    }
    let mut list = egui::ScrollArea::vertical().auto_shrink(false);
    if to_end {
        list = list.vertical_scroll_offset(10_000.0);
    }
    list.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 16.0;
        let total = ui.available_width();
        ui.add(
            crate::components::para(
                    tr!("Apps on Dash Platform you allowed to sign in as your identity. Each connection has its own spending limit and period; the app cannot spend more."),
                    14.0,
                    Weight::Regular,
                    p.muted,
                )
                .line_height(21.0),
        );
        // The demo's example: a real app gives a real link.
        #[cfg(demo_core)]
        if cx.snap.demo
            && crate::components::text_action(ui, a, "link", tr!("Show an example")).clicked()
        {
            cx.send(evolite_api::Command::OpenAppLink {
                text: EXAMPLE.to_string(),
            });
        }
        if let Some(done) = requests.connected.clone() {
            let answer = notice_closable(ui, a, "confirmed", &done, "", None);
            if answer.closed {
                requests.connected = None;
            }
        }
        if wallet.apps.is_empty() {
            card(&p, Margin::ZERO).show(ui, |ui| {
                ui.set_width(inner(total, 0));
                ui.add_space(32.0);
                // With the next step (designer review №17): the app's link is the way in.
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    ui.label(txt(tr!("No connected apps"), 14.0, Weight::Regular, p.muted));
                    ui.label(txt(
                        tr!("To connect one, paste the link the app shows."),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    if button(ui, a, tr!("Paste a link"), ButtonKind::Secondary, None).clicked() {
                        requests.pasting = true;
                    }
                });
                ui.add_space(32.0);
            });
        }
        for (title, apps) in [
            (format!(tr!("Connected · {}"), connected.len()), &connected),
            (format!(tr!("Disconnected · {}"), disconnected.len()), &disconnected),
        ] {
            if apps.is_empty() {
                continue;
            }
            ui.label(txt(
                &title,
                13.0,
                Weight::SemiBold,
                p.muted,
            ));
            for app in apps.iter() {
                let answer = app_card(ui, a, app, &who, now);
                let label = if app.label.is_empty() {
                    app_label(&app.name)
                } else {
                    app.label.clone()
                };
                if answer.disconnect {
                    // It disables keys and costs credits: asked once more first.
                    requests.disconnecting = Some((app.id.clone(), label.clone()));
                    requests.disconnect_result = None;
                }
                if answer.renew {
                    let line = app
                        .session
                        .first()
                        .map(|k| spent_line(k, now))
                        .unwrap_or_default();
                    requests.renewing = Some((label.clone(), line));
                }
                if answer.remove {
                    requests.removing = Some((app.id.clone(), label));
                }
            }
        }
    });
    if let Some((app, line)) = requests.renewing.clone() {
        let mut closed = false;
        match connect2::renew_how(ui, a, &app, &line, &mut closed) {
            Some(true) => requests.pasting = true,
            Some(false) => scan_now = true,
            None => {}
        }
        if closed {
            requests.renewing = None;
        }
    }
    if scan_now {
        requests.scanner.open_menu(ui.ctx());
    }
    if let Some((id, app)) = requests.removing.clone() {
        let mut closed = false;
        if connect2::remove_dialog(ui, a, &app, &mut closed) {
            cx.send(evolite_api::Command::ForgetApp { id });
        }
        if closed {
            requests.removing = None;
        }
    }
    app_requests::show(ui, a, cx, requests);
    app_requests::disconnect_dialog(ui, a, cx, requests);
}

/// What a QR brings to «Apps» (UC-4, AC-T8): an app's link, else why not (`None`: the file
/// dialog was closed, nothing to say).
fn app_link_from_qr(scanned: &crate::qr_scan::Scanned) -> Result<String, Option<&'static str>> {
    use crate::qr_scan::{Kind, Scanned};
    match scanned {
        Scanned::Found {
            text,
            kind: Kind::AppLink,
        } => Ok(text.clone()),
        Scanned::Found { .. } | Scanned::NotDash => {
            Err(Some(tr!("This is not a Dash app request")))
        }
        other => Err(crate::components::qr_error(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{AppBinding, CoreError, KeyLevel};

    #[test]
    fn only_an_app_link_from_a_qr_goes_to_the_core() {
        use crate::qr_scan::{Kind, Scanned};
        let found = |text: &str, kind| Scanned::Found {
            text: text.into(),
            kind,
        };
        assert_eq!(
            app_link_from_qr(&found("dashpay://connect?v=2", Kind::AppLink)),
            Ok("dashpay://connect?v=2".into())
        );
        for other in [
            found("yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f", Kind::Address),
            found("@bob", Kind::Name),
            Scanned::NotDash,
        ] {
            assert_eq!(
                app_link_from_qr(&other),
                Err(Some("This is not a Dash app request"))
            );
        }
        assert_eq!(
            app_link_from_qr(&Scanned::NoQr),
            Err(Some("There is no QR code in the picture"))
        );
        assert_eq!(app_link_from_qr(&Scanned::Cancelled), Err(None));
    }

    fn key(remaining: Option<i64>, state: SessionKeyState) -> SessionKey {
        SessionKey {
            id: 6,
            level: KeyLevel::High,
            binding: AppBinding::Group { id: "G".into() },
            budget: 50_000_000,
            expires_at: 1_000,
            state,
            remaining,
        }
    }

    fn app(session: Vec<SessionKey>, disconnected_at: Option<i64>) -> ConnectedApp {
        ConnectedApp {
            id: "C".into(),
            name: "@yappr".into(),
            label: "Yappr".into(),
            domain: String::new(),
            last_used: 0,
            session,
            disconnected_at,
        }
    }

    /// The design's table: active, used up (any key at 0), expired (the primary key's end),
    /// disconnected first of all.
    #[test]
    fn a_connection_is_active_used_up_expired_or_disconnected() {
        use SessionKeyState::{Expired, Live};
        assert_eq!(
            state_of(&app(vec![key(Some(1), Live)], None)),
            AppState::Active
        );
        assert_eq!(
            state_of(&app(vec![key(None, Live)], None)),
            AppState::Active,
            "not read"
        );
        assert_eq!(
            state_of(&app(vec![key(Some(1), Live), key(Some(0), Live)], None)),
            AppState::UsedUp
        );
        assert_eq!(
            state_of(&app(vec![key(Some(9), Expired)], None)),
            AppState::Expired(1_000)
        );
        assert_eq!(
            state_of(&app(vec![key(Some(0), Live)], Some(5))),
            AppState::Disconnected
        );
    }

    #[test]
    fn spent_is_what_platform_says_is_gone() {
        assert_eq!(
            spent_fraction(&key(Some(48_000_000), SessionKeyState::Live)),
            0.04
        );
        assert_eq!(spent_fraction(&key(None, SessionKeyState::Live)), 0.0);
        assert_eq!(spent_fraction(&key(Some(0), SessionKeyState::Live)), 1.0);
    }

    #[test]
    fn a_disconnect_closes_its_dialog_only_when_done() {
        let mut requests = AppRequests {
            disconnecting: Some(("C".into(), "@yappr".into())),
            ..Default::default()
        };
        // Not asked yet: an answer is someone else's.
        requests.on_disconnected(Ok(()));
        assert!(requests.disconnecting.is_some());
        requests.disconnect_waiting = true;
        requests.on_disconnected(Err(CoreError::NotEnough));
        assert!(requests.disconnecting.is_some() && requests.disconnect_result.is_some());
        requests.disconnect_waiting = true;
        requests.on_disconnected(Ok(()));
        assert!(requests.disconnecting.is_none());
    }
}
