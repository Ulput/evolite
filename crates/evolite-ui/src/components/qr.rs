//! Reading a QR from a picture (FEAT-0006): the button in a field (`field/qr-button`), its
//! menu (`menu/qr-source`), the line under the field (`field/qr-note`) and the overlay while a
//! picture is dragged over the window. The work itself is `qr_scan`.

use super::prelude::*;
use crate::qr_scan::{Parent, Picture, Scanned, Scanner};

/// The menu's width, as in Figma.
const MENU_W: f32 = 280.0;

/// The line under a field after a scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QrNote {
    /// «Filled in from a QR code: {what}. Check the address before sending.»
    Filled(&'static str),
    /// What went wrong, in red; what was typed stays.
    Error(&'static str),
}

/// The words of a scan that brought nothing a field can take.
pub fn qr_error(scanned: &Scanned) -> Option<&'static str> {
    match scanned {
        Scanned::NoQr => Some(tr!("There is no QR code in the picture")),
        Scanned::NotDash => Some(tr!("This is not a Dash address or an app link")),
        Scanned::NotAPicture => Some(tr!("PNG, JPG or BMP, up to 20 MB")),
        Scanned::Found { .. } | Scanned::Cancelled => None,
    }
}

/// The QR button inside a field (to the right of the text, mirrored in Arabic) with its menu.
/// A Tab stop named «Scan a QR code»; Enter or Space opens the menu.
pub fn qr_button(ui: &mut Ui, a: &mut Assets, id: &str, scanner: &mut Scanner) -> Response {
    let p = a.p;
    // 30 wide; the field keeps its height, the hover square reaches 30 high around it.
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 20.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    named(&resp, egui::WidgetType::Button, true, tr!("Scan a QR code"));
    let square = Rect::from_center_size(rect.center(), vec2(30.0, 30.0));
    let over = resp.hovered() || resp.has_focus() || scanner.menu;
    if over {
        ui.painter()
            .rect_filled(square, CornerRadius::same(8), p.raised);
    }
    if scanner.busy() {
        let spin = Rect::from_center_size(rect.center(), vec2(16.0, 16.0));
        ui.put(spin, egui::Spinner::new().size(16.0).color(p.muted));
    } else {
        let icon = Rect::from_center_size(rect.center(), vec2(18.0, 18.0));
        a.icons
            .icon("scan", if over { p.text } else { p.muted }, &p, 18.0)
            .paint_at(ui, icon);
    }
    let resp = resp.on_hover_text(crate::i18n::t(tr!("QR code from a picture")).into_owned());
    if resp.clicked() && !scanner.busy() {
        if scanner.menu {
            scanner.menu = false;
        } else {
            scanner.open_menu(ui.ctx());
            let by_keys = !ui.input(|i| i.pointer.any_click());
            ui.data_mut(|d| d.insert_temp(menu_id(id).with("keys"), by_keys));
        }
    }
    if scanner.menu {
        qr_menu(ui, a, id, square, &resp, scanner);
    }
    resp
}

/// A button with the words and the same menu, for the head of «Apps».
pub fn qr_menu_under(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    anchor: &Response,
    scanner: &mut Scanner,
) {
    if anchor.clicked() && !scanner.busy() {
        if scanner.menu {
            scanner.menu = false;
        } else {
            scanner.open_menu(ui.ctx());
            let by_keys = !ui.input(|i| i.pointer.any_click());
            ui.data_mut(|d| d.insert_temp(menu_id(id).with("keys"), by_keys));
        }
    }
    if scanner.menu {
        qr_menu(ui, a, id, anchor.rect, anchor, scanner);
    }
}

fn menu_id(id: &str) -> egui::Id {
    egui::Id::new("qr-menu").with(id)
}

/// The menu under `anchor`, its right edge at the anchor's: «From the clipboard» (inactive with
/// its reason written below when the clipboard has no picture), «Choose a picture…» and the
/// promise at the bottom. Arrows move between the items, Escape or a click elsewhere closes it.
fn qr_menu(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    anchor: Rect,
    button: &Response,
    scanner: &mut Scanner,
) {
    let p = a.p;
    let menu = menu_id(id);
    let clip = scanner.clip_picture == Some(true);
    let mut chosen: Option<Picture> = None;
    let area = egui::Area::new(menu)
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(anchor.right() - MENU_W, anchor.bottom() + 8.0))
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin::same(8))
                .shadow(egui::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(if p.dark { 120 } else { 40 }),
                })
                .show(ui, |ui| {
                    let w = MENU_W - 16.0;
                    ui.set_width(w);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let items = [
                        (
                            "paste",
                            tr!("From the clipboard"),
                            if clip || scanner.clip_picture.is_none() {
                                tr!("A screenshot or a picture")
                            } else {
                                tr!("There is no picture in the clipboard")
                            },
                            clip,
                        ),
                        (
                            "image",
                            tr!("Choose a picture…"),
                            tr!("PNG, JPG or BMP, up to 20 MB"),
                            true,
                        ),
                    ];
                    for (i, (icon, title, under, enabled)) in items.into_iter().enumerate() {
                        let sense = if enabled {
                            Sense::click()
                        } else {
                            Sense::hover()
                        };
                        let (row, _) = ui.allocate_exact_size(vec2(w, 52.0), Sense::hover());
                        let resp = ui.interact(row, item_id(menu, i), sense);
                        named(&resp, egui::WidgetType::Button, enabled, title);
                        let focused = resp.has_focus();
                        if enabled && (resp.hovered() || focused) {
                            ui.painter()
                                .rect_filled(row, CornerRadius::same(10), p.raised);
                        }
                        if focused {
                            ui.painter().rect_stroke(
                                row,
                                CornerRadius::same(10),
                                Stroke::new(1.0, p.text),
                                StrokeKind::Inside,
                            );
                        }
                        let (ink, faint) = if enabled {
                            (p.text, p.muted)
                        } else {
                            (p.muted.gamma_multiply(0.6), p.muted.gamma_multiply(0.8))
                        };
                        a.icons.icon(icon, ink, &p, 18.0).paint_at(
                            ui,
                            Rect::from_min_size(
                                pos2(row.left() + 12.0, row.center().y - 9.0),
                                vec2(18.0, 18.0),
                            ),
                        );
                        paint_text(
                            ui,
                            pos2(row.left() + 42.0, row.top() + 9.0),
                            Align2::LEFT_TOP,
                            title,
                            sans(14.0, Weight::Medium),
                            ink,
                        );
                        paint_text(
                            ui,
                            pos2(row.left() + 42.0, row.top() + 29.0),
                            Align2::LEFT_TOP,
                            under,
                            sans(12.0, Weight::Regular),
                            faint,
                        );
                        if enabled && resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            chosen = Some(if i == 0 {
                                Picture::Clipboard
                            } else {
                                Picture::Choose(Parent::now())
                            });
                        }
                    }
                    let (line, _) = ui.allocate_exact_size(vec2(w, 9.0), Sense::hover());
                    ui.painter().hline(
                        line.x_range().shrink(4.0),
                        line.center().y,
                        Stroke::new(1.0, p.line),
                    );
                    let (foot, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                    paint_text(
                        ui,
                        pos2(foot.left() + 12.0, foot.center().y),
                        Align2::LEFT_CENTER,
                        tr!("The picture does not leave this computer"),
                        sans(12.0, Weight::Regular),
                        p.muted,
                    );
                });
        });
    keys(ui, menu, button, clip, scanner);
    let outside = ui.input(|i| {
        i.pointer.any_click()
            && i.pointer
                .interact_pos()
                .is_some_and(|at| !area.response.rect.contains(at) && !anchor.contains(at))
    });
    if outside {
        scanner.menu = false;
    }
    if let Some(picture) = chosen {
        scanner.start(ui.ctx(), picture);
        button.request_focus();
    }
}

fn item_id(menu: egui::Id, i: usize) -> egui::Id {
    menu.with("item").with(i)
}

/// The menu's keys: focus on the first item when it was opened by the keyboard, arrows between
/// the items, Escape back to the button.
fn keys(ui: &Ui, menu: egui::Id, button: &Response, clip: bool, scanner: &mut Scanner) {
    let by_keys = menu.with("keys");
    // The items that take a click, in order: the clipboard only when it has a picture.
    let items: Vec<egui::Id> = [clip, true]
        .into_iter()
        .enumerate()
        .filter(|(_, on)| *on)
        .map(|(i, _)| item_id(menu, i))
        .collect();
    if ui.data(|d| d.get_temp::<bool>(by_keys)).unwrap_or(false) && scanner.clip_picture.is_some() {
        ui.data_mut(|d| d.remove::<bool>(by_keys));
        ui.memory_mut(|m| m.request_focus(items[0]));
    }
    // Taken here: the dialog around the field must not close with it.
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        scanner.menu = false;
        button.request_focus();
        return;
    }
    // egui moves the focus by the arrows itself at the end of the frame, out of the menu: the
    // item the arrow goes to is kept and given the focus until it has it.
    let last_id = menu.with("last-focus");
    let want_id = menu.with("want-focus");
    let n = items.len();
    let focused = ui.memory(|m| m.focused());
    let at = focused.and_then(|f| items.iter().position(|i| *i == f));
    let last = ui.data(|d| d.get_temp::<usize>(last_id)).filter(|i| *i < n);
    let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
    let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
    let mut want = ui.data(|d| d.get_temp::<usize>(want_id)).filter(|i| *i < n);
    if let Some(from) = at.or(last).filter(|_| down || up) {
        want = Some(if down {
            (from + 1) % n
        } else {
            (from + n - 1) % n
        });
    }
    if let Some(w) = want
        && at != Some(w)
    {
        ui.memory_mut(|m| m.request_focus(items[w]));
    }
    ui.data_mut(|d| {
        // Kept through the frame of the arrow itself: egui takes the focus away at its end.
        let keep = want.filter(|w| down || up || at != Some(*w));
        for (id, value) in [(want_id, keep), (last_id, want.or(at))] {
            if let Some(v) = value {
                d.insert_temp(id, v);
            } else {
                d.remove::<usize>(id);
            }
        }
    });
}

/// The line under a field: a tick and muted words, or a warning in red.
pub fn qr_note(ui: &mut Ui, a: &mut Assets, note: &QrNote) {
    let p = a.p;
    let (icon, color, line) = match note {
        // Whole sentences: the catalogs have no single word «address», only its plural forms
        // (QA pass, 2026-11-10: «Заполнено из QR-кода: address»).
        QrNote::Filled(what) => (
            "check",
            p.muted,
            match *what {
                "address" => {
                    tr!("Filled in from a QR code: the address. Check the address before sending")
                        .to_owned()
                }
                "address and amount" => tr!(
                    "Filled in from a QR code: the address and the amount. Check the address before sending"
                )
                .to_owned(),
                "name" => {
                    tr!("Filled in from a QR code: the name. Check the address before sending")
                        .to_owned()
                }
                "name and amount" => tr!(
                    "Filled in from a QR code: the name and the amount. Check the address before sending"
                )
                .to_owned(),
                other => format!(
                    tr!("Filled in from a QR code: {}. Check the address before sending"),
                    crate::i18n::t(other)
                ),
            },
        ),
        QrNote::Error(words) => ("warning", p.danger, words.to_string()),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add(a.icons.icon(icon, color, &p, 14.0));
        ui.add(crate::components::para(&line, 12.0, Weight::Regular, color));
    });
}

/// While a picture is dragged over the window: `rect` covered, dashed frame, what dropping does.
pub fn drop_overlay(ctx: &egui::Context, a: &mut Assets, rect: Rect) {
    egui::Area::new(egui::Id::new("qr-drop"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .interactable(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::hover());
            paint_drop(ui, a, rect);
        });
}

fn paint_drop(ui: &Ui, a: &mut Assets, rect: Rect) {
    let p = a.p;
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(16), p.surface.gamma_multiply(0.92));
    let inner = rect.shrink(1.0);
    let corners = [
        inner.left_top(),
        inner.right_top(),
        inner.right_bottom(),
        inner.left_bottom(),
        inner.left_top(),
    ];
    for pair in corners.windows(2) {
        painter.extend(egui::Shape::dashed_line(
            pair,
            Stroke::new(2.0, p.text),
            8.0,
            6.0,
        ));
    }
    let c = rect.center();
    a.icons.icon("image", p.text, &p, 32.0).paint_at(
        ui,
        Rect::from_center_size(c - vec2(0.0, 36.0), vec2(32.0, 32.0)),
    );
    painter.text(
        c + vec2(0.0, 6.0),
        Align2::CENTER_CENTER,
        crate::i18n::t(tr!("Drop the picture to read the QR")),
        sans(16.0, Weight::SemiBold),
        p.text,
    );
    painter.text(
        c + vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        crate::i18n::t(&format!(
            "{} · {}",
            tr!("PNG, JPG or BMP, up to 20 MB"),
            tr!("The picture does not leave this computer")
        )),
        sans(12.0, Weight::Regular),
        p.muted,
    );
}
