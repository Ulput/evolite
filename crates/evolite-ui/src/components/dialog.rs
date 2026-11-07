//! A modal dialog: surface, radius 20, padding 24 × 28; icon box, title and a close button on
//! top. The page behind is dimmed.

use super::prelude::*;

/// Shows the dialog and runs `body`, which returns true to close it. Returns true when the
/// dialog should close (by the body, the cross, Escape or a click outside).
///
/// The keyboard (the designer's rule b′, 2026-11-06): when it opens, the focus is in its first
/// field; without fields, on the primary button of its [`super::button_row`], or on «Cancel»
/// with its ring shown when the dialog's action cannot be undone (a Danger button, or a
/// [`super::button_row_destructive`]). Enter in a field presses the primary; in a destructive
/// dialog it never presses anything and goes on to the next field.
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
    dialog_with_header_held(ui, a, id, width, false, header, body)
}

/// [`dialog_with_header`] that Escape and a click outside do not close while `held`: something
/// inside is being edited and Escape is its own (FEAT-0003: a note edited in an operation's
/// card). The cross and the body still close it.
pub fn dialog_with_header_held(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    width: f32,
    held: bool,
    header: impl FnOnce(&mut Ui, &mut Assets),
    body: impl FnOnce(&mut Ui, &mut Assets) -> bool,
) -> bool {
    let p = a.p;
    let id = egui::Id::new(id);
    let ctx = ui.ctx().clone();
    let place = Place::of(&ctx, id);
    let mut modal = egui::Modal::new(id);
    if let Some(top) = place.top {
        modal =
            modal.area(egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, top)));
    }
    let modal = modal
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
                let cross = ui
                    .add(egui::Button::image(img).frame(false))
                    .on_hover_text(crate::i18n::t(tr!("Close")).into_owned());
                // A picture alone says nothing to a screen reader (BUG-41).
                named(&cross, egui::WidgetType::Button, true, tr!("Close"));
                if cross.clicked() {
                    close = true;
                }
            });
        });
        // Where the focus starts and what Enter does (the designer's rule b′): the first field
        // takes the focus when the dialog opens, else its row of buttons places it
        // ([`super::button_row`]).
        let outer = crate::a11y::dialog_begin(ui.ctx(), id);
        let done = body(ui, a);
        crate::a11y::dialog_end(ui.ctx(), outer);
        done || close
    });
    place.keep(&ctx, id, response.response.rect);
    response.inner || (!held && response.should_close())
}

/// Where an open dialog stands. It opens in the middle of the window; from its second frame its
/// top stays put and it grows or shrinks downwards, so a line of error or a hint that comes and
/// goes does not move the field under the pointer (BUG-39 №3). A new opening or another window
/// size centres it again.
#[derive(Clone, Copy, Debug)]
struct Place {
    /// The top, from the window's top, once it is kept.
    top: Option<f32>,
    /// Frames it has been shown in a row.
    frames: u8,
    /// The window it was placed in.
    window: Rect,
    /// The pass it was last shown in.
    pass: u64,
}

impl Place {
    /// The dialog's place this frame: as last frame if it was open then, in the same window.
    fn of(ctx: &egui::Context, id: egui::Id) -> Self {
        let pass = ctx.cumulative_pass_nr();
        let window = ctx.content_rect();
        ctx.data(|d| d.get_temp::<Place>(id.with("place")))
            .filter(|p| p.pass + 1 >= pass && p.window == window)
            .unwrap_or(Place {
                top: None,
                frames: 0,
                window,
                pass,
            })
    }

    /// Remembers where the dialog was drawn; its top is kept from the second frame on (the
    /// first one only measures it).
    fn keep(mut self, ctx: &egui::Context, id: egui::Id, drawn: Rect) {
        self.frames = self.frames.saturating_add(1);
        if self.top.is_none() && self.frames >= 2 {
            self.top = Some(drawn.top() - self.window.top());
        }
        self.pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| d.insert_temp(id.with("place"), self));
    }
}
