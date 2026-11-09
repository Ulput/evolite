//! 10 · Apps: apps allowed to sign in as the identity, and the three ways an app can
//! ask the wallet for something. The list comes from the core; "Disconnect" asks the core to
//! remove the app's key.

use super::app_requests::{self, AppRequests};
use super::txt;
use crate::components::{
    Assets, ButtonKind, GridSpec, button, button_enabled, card, grid, inner, page_header,
    paint_letter_box,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::{APP_CARD_MIN_W, Weight, mono, sans};
use chrono::{DateTime, Local};
use egui::{Align2, Margin, Rect, Sense, Ui, pos2, vec2};
use evolite_api::ConnectedApp;

/// An example link for each way, so the dialogs can be seen without a real app (the demo).
#[cfg(demo_core)]
const EXAMPLES: [&str; 3] = [
    "dash-key:yappr?contract=6Hk2Ft9wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q&key=MEDIUM",
    "dash-kx:jembe?session=8f3a",
    "dash-st:jembe?transition=0201a4c3e9f0",
];

const WAYS: [(&str, &str, &str); 3] = [
    (
        "app-connect",
        "App Connect",
        tr!(
            "The app shows a QR or a dash-key: link — evolite answers the sign-in, and by a second dash-st: link adds the app's keys to the identity."
        ),
    ),
    (
        "key",
        tr!("Sign in by code"),
        tr!("A key exchange over an encrypted channel; the codes on both screens must match."),
    ),
    (
        "sign",
        tr!("Signing by link"),
        tr!(
            "dash-st: — the app asks to sign one operation; you see exactly what and what it costs."
        ),
    ),
];

/// "today, 14:20", "yesterday", "3 days ago", then the date.
fn last_used(at: i64, now: DateTime<Local>) -> String {
    let days = (now.timestamp() - at) / 86_400;
    match days {
        2..=6 => text::plural(days as u32, tr!("day ago"), tr!("days ago")),
        _ => text::when(at, now),
    }
}

/// What the app may do: the identity, its key and the contracts it signs for.
fn access(app: &ConnectedApp, identity: Option<&str>) -> String {
    let who = identity.map_or_else(|| "identity".to_string(), |n| format!("@{n}"));
    match &app.key {
        Some((id, level, contracts)) => {
            let docs = match contracts.len() {
                // A key without a contract binding: documents of any app.
                0 => tr!("documents of any app").to_string(),
                1 => format!(tr!("documents of contract {}"), contracts[0]),
                _ => format!(tr!("{}'s documents"), contracts.join(", ")),
            };
            format!(
                tr!("{who} · key #{id} {} · {docs}"),
                text::key_level(*level),
                who = who,
                id = id,
                docs = docs
            )
        }
        None => format!(
            tr!("{who} · the app's keys are not on the identity yet"),
            who = who
        ),
    }
}

pub fn apps(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, requests: &mut AppRequests) {
    let p = a.p;
    let wallet = cx.wallet();
    let name = wallet.identity.as_ref().and_then(|i| i.name());
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

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 22.0;
        let total = ui.available_width();
        ui.add(
            egui::Label::new(
                txt(
                    tr!("Dash Platform apps you let sign in as your identity. An app's key signs documents in your name (fees come from the credits), but cannot transfer credits or DASH."),
                    14.0,
                    Weight::Regular,
                    p.muted,
                )
                .line_height(Some(21.0)),
            )
            .wrap(),
        );

        card(&p, Margin::ZERO).show(ui, |ui| {
            ui.set_width(inner(total, 0));
            ui.spacing_mut().item_spacing.y = 0.0;
            if wallet.apps.is_empty() {
                ui.add_space(32.0);
                // With the next step (designer review №17): the app's link is the way in.
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    ui.label(txt(
                        tr!("No connected apps"),
                        14.0,
                        Weight::Regular,
                        p.muted,
                    ));
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
            }
            for (i, app) in wallet.apps.iter().enumerate() {
                if i > 0 {
                    let (line, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().rect_filled(line, 0, p.line);
                }
                let who = name.map_or_else(|| "identity".to_string(), |n| format!("@{n}"));
                if app_row(
                    ui,
                    a,
                    app,
                    &access(app, name),
                    &last_used(app.last_used, now),
                    &who,
                ) {
                    // It disables keys and costs credits: asked once more first.
                    requests.disconnecting = Some((app.id.clone(), app.name.clone()));
                    requests.disconnect_result = None;
                }
            }
        });

        // Three explanations side by side while each is at least an app card wide, else two,
        // else one under another (responsive spec R4). The cards of a row are as tall as the
        // tallest one (designer review №17) in the very frame they are drawn: no height is
        // kept from the last frame and no frame is asked for again.
        let spec = GridSpec {
            min_w: APP_CARD_MIN_W,
            gap: vec2(16.0, 16.0),
            max_cols: 3,
            even: false,
        };
        grid(ui, WAYS.len(), spec, |ui, i, w, cell| {
            let (icon, title, text) = WAYS[i];
            #[cfg(demo_core)]
            let example = EXAMPLES[i];
            cell.card(ui, card(&p, Margin::same(18)), |ui| {
                ui.set_width(inner(w, 18));
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.add(a.icons.icon(icon, p.text, &p, 18.0));
                    ui.label(txt(title, 14.0, Weight::SemiBold, p.text));
                });
                ui.add(
                    egui::Label::new(
                        txt(text, 12.0, Weight::Regular, p.muted).line_height(Some(18.0)),
                    )
                    .wrap(),
                );
                // The examples are the demo's: a real app gives a real link.
                #[cfg(demo_core)]
                if cx.snap.demo
                    && crate::components::text_action(ui, a, "link", tr!("Show an example"))
                        .clicked()
                {
                    cx.send(evolite_api::Command::OpenAppLink {
                        text: example.to_string(),
                    });
                }
            });
        });
    });
    app_requests::show(ui, a, cx, requests);
    app_requests::disconnect_dialog(ui, a, cx, requests);
}

/// One connected app: logo letter, name and domain, what it may do, last sign-in, "Disconnect".
/// `who`: the identity it signs in as. Returns true when "Disconnect" was clicked.
fn app_row(
    ui: &mut Ui,
    a: &mut Assets,
    app: &ConnectedApp,
    access: &str,
    last: &str,
    who: &str,
) -> bool {
    let p = a.p;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 77.0), Sense::hover());
    let cy = rect.center().y;
    let logo = Rect::from_min_size(pos2(rect.left() + 20.0, cy - 22.0), vec2(44.0, 44.0));
    paint_letter_box(ui, &p, logo, &app.name);
    let painter = ui.painter();
    let x = logo.right() + 16.0;
    let name = painter.text(
        pos2(x, cy - 2.0),
        Align2::LEFT_BOTTOM,
        crate::i18n::t(&app.name),
        sans(15.0, Weight::SemiBold),
        p.text,
    );
    painter.text(
        pos2(name.right() + 8.0, name.center().y),
        Align2::LEFT_CENTER,
        crate::i18n::t(&app.domain),
        mono(12.0, Weight::Regular),
        p.muted,
    );
    painter.text(
        pos2(x, cy + 2.0),
        Align2::LEFT_TOP,
        crate::i18n::t(access),
        sans(13.0, Weight::Regular),
        p.muted,
    );

    let button_rect = Rect::from_min_size(
        pos2(rect.right() - 20.0 - 112.0, cy - 20.5),
        vec2(112.0, 41.0),
    );
    let last_x = button_rect.left() - 16.0;
    painter.text(
        pos2(last_x, cy - 1.0),
        Align2::RIGHT_BOTTOM,
        crate::i18n::t(tr!("last sign-in")),
        sans(11.0, Weight::Regular),
        p.muted,
    );
    painter.text(
        pos2(last_x, cy + 1.0),
        Align2::RIGHT_TOP,
        crate::i18n::t(last),
        sans(13.0, Weight::Medium),
        p.text,
    );
    // A child UI: placing the button must not move the parent's cursor.
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button_rect));
    let ui = &mut child;
    button(ui, a, tr!("Disconnect"), ButtonKind::Ghost, Some(112.0))
        .on_hover_text(
            crate::i18n::t(&format!(
                tr!("{} will no longer be able to sign in as {who}"),
                app.name,
                who = who
            ))
            .into_owned(),
        )
        .clicked()
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

    #[test]
    fn only_an_app_link_from_a_qr_goes_to_the_core() {
        use crate::qr_scan::{Kind, Scanned};
        let found = |text: &str, kind| Scanned::Found {
            text: text.into(),
            kind,
        };
        assert_eq!(
            app_link_from_qr(&found("dash-key:yappr", Kind::AppLink)),
            Ok("dash-key:yappr".into())
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
    use evolite_api::{CoreError, KeyLevel};

    fn app(key: Option<(u32, KeyLevel, Vec<String>)>) -> ConnectedApp {
        ConnectedApp {
            id: "C".into(),
            name: "@yappr".into(),
            domain: String::new(),
            key,
            last_used: 0,
        }
    }

    #[test]
    fn says_what_an_app_key_can_sign() {
        // The real core's keys are unbound: not "only its documents".
        assert_eq!(
            access(&app(Some((6, KeyLevel::High, Vec::new()))), Some("bob")),
            format!(
                "@bob · key #6 {} · documents of any app",
                text::key_level(KeyLevel::High)
            )
        );
        assert!(
            access(
                &app(Some((4, KeyLevel::Medium, vec!["jembe".into()]))),
                None
            )
            .ends_with("documents of contract jembe")
        );
        assert_eq!(
            access(&app(None), Some("bob")),
            "@bob · the app's keys are not on the identity yet"
        );
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
