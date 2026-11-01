//! A modal dialog: surface, radius 20, padding 24 × 28; icon box, title and a close button on
//! top. The page behind is dimmed.

use super::prelude::*;

/// Shows the dialog and runs `body`, which returns true to close it. Returns true when the
/// dialog should close (by the body, the cross, Escape or a click outside).
pub fn dialog(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    icon: &str,
    title: &str,
    width: f32,
    body: impl FnOnce(&mut Ui, &mut Assets) -> bool,
) -> bool {
    dialog_with_header(
        ui,
        a,
        id,
        width,
        |ui, a| {
            let p = a.p;
            let (boxr, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
            super::paint_icon_box(ui, a, boxr, 12, icon, 20.0, p.text);
            ui.label(txt(title, 20.0, Weight::Bold, p.text));
        },
        body,
    )
}

/// [`dialog()`] with its own header on the left of the close button (a person's picture and
/// name instead of an icon and a title).
pub fn dialog_with_header(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    width: f32,
    header: impl FnOnce(&mut Ui, &mut Assets),
    body: impl FnOnce(&mut Ui, &mut Assets) -> bool,
) -> bool {
    let p = a.p;
    let modal = egui::Modal::new(egui::Id::new(id))
        .frame(
            Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(CornerRadius::same(20))
                .inner_margin(Margin::symmetric(28, 24)),
        )
        .backdrop_color(Color32::from_black_alpha(if p.dark { 160 } else { 90 }));
    let response = modal.show(ui.ctx(), |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 16.0;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            header(ui, a);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let img = a.icons.icon("close", p.muted, &p, 20.0);
                if ui
                    .add(egui::Button::image(img).frame(false))
                    .on_hover_text(crate::i18n::t(tr!("Close")).into_owned())
                    .clicked()
                {
                    close = true;
                }
            });
        });
        body(ui, a) || close
    });
    response.inner || response.should_close()
}
