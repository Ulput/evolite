//! A modal dialog: surface, radius 20, padding 24 × 28; icon box, title and a close button on
//! top. The page behind is dimmed.
//!
//! However low its window, a dialog stays in it (BUG-122): the header stays at the top, its
//! footer (the password, the line of error under it, the row of buttons) at the bottom, and the
//! middle between them scrolls, with a fade at its bottom while there is more below. A dialog
//! that fits looks as before: no bar, no fade.

use super::prelude::*;

/// The two parts of a dialog with a footer ([`dialog_with_footer`]): its body is called once
/// for each, the middle first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// Everything between the header and the footer: it scrolls when the window is too low.
    Middle,
    /// The password, its line of error and the row of buttons: always in the window.
    Footer,
}

/// Shows the dialog and runs `body`, which returns true to close it. Returns true when the
/// dialog should close (by the body, the cross, Escape or a click outside). The whole body is
/// the dialog's middle: a dialog with a password, a line of error or buttons uses
/// [`dialog_with_footer`], so that they are never out of the window.
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
    dialog_with_footer(ui, a, id, icon, title, width, middle_only(body))
}

/// [`dialog()`] whose `body` is called for its [`Part::Middle`], then for its [`Part::Footer`];
/// either returns true to close it. The footer is never scrolled (BUG-122).
///
/// The body runs twice a frame: outside its branch on `part` only lay out (spacing, values
/// read). What acts — a command sent, a value taken out of the state, a write to egui's memory —
/// goes inside the branch of its part, or it happens twice (the architect, BUG-122 r1 P3 1).
pub fn dialog_with_footer(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    icon: &str,
    title: &str,
    width: f32,
    body: impl FnMut(&mut Ui, &mut Assets, Part) -> bool,
) -> bool {
    header_dialog(
        ui,
        a,
        id,
        width,
        false,
        Align::Center,
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
    dialog_with_header_and_footer(ui, a, id, width, header, middle_only(body))
}

/// [`dialog_with_header`] with a footer, as [`dialog_with_footer`].
pub fn dialog_with_header_and_footer(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    width: f32,
    header: impl FnOnce(&mut Ui, &mut Assets),
    body: impl FnMut(&mut Ui, &mut Assets, Part) -> bool,
) -> bool {
    header_dialog(ui, a, id, width, false, Align::Center, header, body)
}

/// [`dialog_with_header_and_footer`] that Escape and a click outside do not close while `held`:
/// something inside is being edited and Escape is its own (FEAT-0003: a note edited in an
/// operation's card). The cross and the body still close it.
pub fn dialog_with_header_held(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    width: f32,
    held: bool,
    header: impl FnOnce(&mut Ui, &mut Assets),
    body: impl FnMut(&mut Ui, &mut Assets, Part) -> bool,
) -> bool {
    // The operation's card, its only user: a title of two lines keeps the cross at the top
    // (BUG-114).
    header_dialog(ui, a, id, width, held, Align::Min, header, body)
}

/// A body without a footer: all of it is the middle.
fn middle_only(
    body: impl FnOnce(&mut Ui, &mut Assets) -> bool,
) -> impl FnMut(&mut Ui, &mut Assets, Part) -> bool {
    let mut body = Some(body);
    move |ui, a, part| match part {
        Part::Middle => body.take().is_some_and(|body| body(ui, a)),
        Part::Footer => false,
    }
}

/// The next frame shows the dialog's middle from its top: it shows another thing now (another
/// operation in the same card). A new opening always does.
pub fn dialog_middle_to_top(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(id).with("to-top"), true));
}

/// This frame the dialog's middle shows its end and keeps to it (the dev presets that show what
/// is at the end of a long dialog).
pub fn dialog_middle_at_end(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(id).with("to-end"), true));
}

/// The dialog drawn last (for tests).
#[cfg(all(test, demo_core, feature = "dev-tools"))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DialogSeen {
    pub id: egui::Id,
    /// The pass it was drawn in.
    pub pass: u64,
    /// The whole dialog.
    pub rect: Rect,
    /// Its footer, when it has one.
    pub footer: Option<Rect>,
    /// The visible part of its middle, when the middle had more than it showed.
    pub scrolled: Option<Rect>,
    /// How far its middle is scrolled down.
    pub offset: f32,
    /// The fade at the bottom of its middle: there was more below.
    pub fade: Option<Rect>,
    /// The AccessKit ids its widgets descend from (its `Ui` and its layer): what is on the
    /// page behind it does not.
    pub nodes: [u64; 2],
}

#[cfg(all(test, demo_core, feature = "dev-tools"))]
fn seen_id() -> egui::Id {
    egui::Id::new("evolite-dialog-seen")
}

/// The dialog drawn in the last frame (for tests); `None` when none was.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
pub(crate) fn dialog_seen(ctx: &egui::Context) -> Option<DialogSeen> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<DialogSeen>(seen_id()))
        .filter(|s| s.pass + 1 >= pass)
}

/// The dialogs with a header: the cross stands at `cross` of the header's height.
#[allow(clippy::too_many_arguments)]
fn header_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    id: &str,
    width: f32,
    held: bool,
    cross: Align,
    header: impl FnOnce(&mut Ui, &mut Assets),
    mut body: impl FnMut(&mut Ui, &mut Assets, Part) -> bool,
) -> bool {
    let p = a.p;
    let id = egui::Id::new(id);
    let ctx = ui.ctx().clone();
    let place = Place::of(&ctx, id);
    let mut modal = egui::Modal::new(id);
    // A dialog whose middle scrolled last frame is as high as the window less its margins: it
    // stands [`MARGIN_SCROLL`] from the top (BUG-122).
    let scrolling = ctx
        .data(|d| d.get_temp::<Option<Rect>>(id.with("middle-rect")))
        .flatten()
        .is_some();
    if scrolling && !place.opened {
        modal = modal.area(
            egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, MARGIN_SCROLL)),
        );
    } else if let Some(top) = place.top {
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
        ui.spacing_mut().item_spacing.y = GAP;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            header(ui, a);
            ui.with_layout(Layout::right_to_left(cross), |ui| {
                if close_cross(ui, a, CROSS).clicked() {
                    close = true;
                }
            });
        });
        // Where the focus starts and what Enter does (the designer's rule b′): the first field
        // takes the focus when the dialog opens, else its row of buttons places it
        // ([`super::button_row`]), in the footer: both parts are inside.
        let outer = crate::a11y::dialog_begin(ui.ctx(), id);
        let asked = ui.data_mut(|d| d.remove_temp::<bool>(id.with("to-top")).is_some());
        if place.opened {
            // A new opening starts with no focus followed yet (see [`follow_focus`]).
            ui.data_mut(|d| d.remove_temp::<(Option<egui::Id>, u8)>(id.with("focus-seen")));
        }
        // The focus is followed once the dialog has been drawn where it stands: what the first
        // (measuring) frame left of a widget is no place to follow.
        let settled = place.frames >= 2;
        close |= middle(ui, a, id, (place.opened || asked, settled), &mut body);
        let footer_top = ui.cursor().top();
        close |= body(ui, a, Part::Footer);
        // The footer's height with the dialog's gap above it, for the next frame's middle. Not
        // up to where the next thing would go: a body's own spacing (14 in «Check the
        // payment») is not the gap the dialog leaves between its middle and its footer.
        let drawn = ui.min_rect().bottom() - footer_top;
        let footer = if drawn > 0.0 { drawn + GAP } else { 0.0 };
        ui.data_mut(|d| d.insert_temp(id.with("footer"), footer));
        crate::a11y::dialog_end(ui.ctx(), outer);
        #[cfg(all(test, demo_core, feature = "dev-tools"))]
        {
            let drawn = ui.min_rect();
            let seen = DialogSeen {
                id,
                pass: ui.ctx().cumulative_pass_nr(),
                rect: drawn,
                footer: (drawn.bottom() > footer_top)
                    .then(|| Rect::from_min_max(pos2(drawn.left(), footer_top), drawn.max)),
                scrolled: ui
                    .data(|d| d.get_temp::<Option<Rect>>(id.with("middle-rect")))
                    .flatten(),
                offset: ui
                    .data(|d| d.get_temp::<f32>(id.with("offset")))
                    .unwrap_or(0.0),
                fade: ui
                    .data(|d| d.get_temp::<Option<Rect>>(id.with("fade")))
                    .flatten(),
                nodes: [ui.id().value(), ui.layer_id().id.value()],
            };
            ui.data_mut(|d| d.insert_temp(seen_id(), seen));
        }
        close
    });
    place.keep(&ctx, id, response.response.rect);
    response.inner || (!held && response.should_close())
}

/// The dialog's middle: in a scroll area as high as the window leaves it between the header
/// above and the footer as high as it was last frame. Whether the body closed the dialog.
fn middle(
    ui: &mut Ui,
    a: &mut Assets,
    id: egui::Id,
    (to_top, settled): (bool, bool),
    body: &mut impl FnMut(&mut Ui, &mut Assets, Part) -> bool,
) -> bool {
    /// The fade over the bottom of a middle that goes on (the designer, BUG-122 item 4).
    const FADE: f32 = 16.0;
    // The first frame only measures: the whole middle, unscrolled. Still in the same area, so
    // that its widgets keep their ids and the focus asked for on opening stays where it went.
    let sizing = ui.is_sizing_pass();
    if sizing && to_top {
        // To the top once it is drawn: the next frame.
        ui.data_mut(|d| d.insert_temp(id.with("to-top"), true));
    }
    let room = if sizing {
        f32::INFINITY
    } else {
        middle_room(ui, id)
    };
    // The lines are as wide with a bar as without: egui widens an area to its content, so the
    // bar's gutter goes into the dialog's side padding. In a child [`Ui`], or the dialog would
    // grow by the gutter (BUG-90 r2).
    let width = ui.available_width();
    // As high as `room`, not as the dialog was last frame (an area's `Ui` is no higher than
    // that): a middle that grows, a line of error come, grows with it. The first frame measures
    // it as high as it is.
    let gutter = ui.spacing().scroll.allocated_width();
    let mut wider = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id.with("middle-place"))
            .max_rect(Rect::from_min_size(
                ui.cursor().min,
                vec2(width + gutter, room),
            )),
    );
    let mut area = egui::ScrollArea::vertical()
        .id_salt(id.with("middle"))
        .auto_shrink([false, true])
        .max_height(room);
    if to_top {
        area = area.vertical_scroll_offset(0.0);
    }
    if ui.data_mut(|d| d.remove_temp::<bool>(id.with("to-end")).is_some()) {
        area = area.stick_to_bottom(true);
    }
    let shown = area.show(&mut wider, |ui| {
        ui.set_max_width(width);
        let close = body(ui, a, Part::Middle);
        if settled {
            follow_focus(ui, id);
        }
        close
    });
    let used = wider.min_rect();
    ui.advance_cursor_after_rect(Rect::from_min_size(used.min, vec2(width, used.height())));
    // More below: a fade into the dialog's colour, gone once it is scrolled to its end.
    let view = shown.inner_rect;
    let fade = super::more_below(&shown).then(|| super::fade_up(ui, view, a.p.surface, FADE));
    let scrolls = shown.content_size.y > view.height() + 0.5;
    ui.data_mut(|d| {
        d.insert_temp(id.with("fade"), fade);
        d.insert_temp(id.with("middle-rect"), scrolls.then_some(view));
        d.insert_temp(id.with("offset"), shown.state.offset.y);
        // How high the middle is whole, for the next frame's choice of margin.
        d.insert_temp(id.with("content"), shown.content_size.y);
    });
    shown.inner
}

/// Between the header, the middle and the footer.
const GAP: f32 = 16.0;

/// From the window's top and bottom edges to a dialog that fits whole: it may come this close
/// and not scroll (the ordinary card at 1024×768 stands 14 from them in English, 5 in ru, uk,
/// es, fr and pt-BR; BUG-90).
pub const MARGIN_FIT: f32 = 4.0;
/// From the window's top and bottom edges to a dialog that scrolls: the screen behind shows
/// around it (the designer, BUG-122: at 4 its corners would touch the edge).
pub const MARGIN_SCROLL: f32 = 16.0;

/// The most the middle can take: the window less the dialog's margins, the header drawn above it
/// and the footer under it as it was last frame (BUG-90's card, now every dialog's). A middle
/// that fits within [`MARGIN_FIT`] takes it whole; one that does not scrolls within
/// [`MARGIN_SCROLL`].
fn middle_room(ui: &Ui, id: egui::Id) -> f32 {
    const PADDING: f32 = 50.0; // the dialog's 24 above and below, and its stroke
    const FIRST_FRAME_FOOTER: f32 = 56.0;
    const AT_LEAST: f32 = 96.0;
    let (footer, content) = ui.data(|d| {
        (
            d.get_temp::<f32>(id.with("footer"))
                .unwrap_or(FIRST_FRAME_FOOTER),
            d.get_temp::<f32>(id.with("content")),
        )
    });
    let above = ui.cursor().top() - ui.min_rect().top();
    let window = ui.ctx().content_rect().height();
    let room = |margin: f32| (window - 2.0 * margin - PADDING - above - footer).max(AT_LEAST);
    let fit = room(MARGIN_FIT);
    if content.is_some_and(|whole| whole <= fit + 0.5) {
        fit
    } else {
        room(MARGIN_SCROLL)
    }
}

/// Brings into the visible part of the middle what the focus has just come to, so that Tab
/// never lands on a field below the fold and Enter never presses it blind. Called at the end of
/// the middle; only a widget in the middle moves it, which nothing in the header or the footer
/// is. For a few frames after a change of focus only — what the focus brings may change the
/// footer and so the middle's room («Add a note» opens its field and the card's buttons change)
/// — so the wheel can still leave a focused widget behind.
fn follow_focus(ui: &Ui, id: egui::Id) {
    const AIR: f32 = 12.0; // a field's frame comes into view with it
    /// Frames a new focus is kept in view while the dialog settles around it.
    const FOLLOW: u8 = 4;
    let focus = ui.memory(|m| m.focused());
    let key = id.with("focus-seen");
    let Some((seen, left)) = ui.data(|d| d.get_temp::<(Option<egui::Id>, u8)>(key)) else {
        // The focus the dialog opened with (rule b′: its first field) is where it stands: a
        // new opening shows its middle from the top (the designer, BUG-122; App Connect shows
        // whose request it is first).
        ui.data_mut(|d| d.insert_temp(key, (focus, 0u8)));
        return;
    };
    let left = if focus == seen { left } else { FOLLOW };
    if left == 0 {
        return;
    }
    let widget = focus.and_then(|f| ui.ctx().read_response(f));
    // A widget that has just come (the note field «Add a note» opens) is not drawn yet: it is
    // looked at next frame, its frames not counted.
    if focus.is_some() && widget.is_none() {
        ui.data_mut(|d| d.insert_temp(key, (seen, 0)));
        return;
    }
    ui.data_mut(|d| d.insert_temp(key, (focus, left - 1)));
    let Some(widget) = widget else {
        return;
    };
    // In the middle and cut by its edge, or in it with its frame cut (a field's 10 around its
    // text): what is in the header or the footer is never in the middle's view.
    let view = ui.clip_rect();
    let cut = widget.interact_rect != widget.rect
        || (view.intersects(widget.rect)
            && !view.contains_rect(widget.rect.expand2(vec2(0.0, AIR))));
    if cut {
        crate::diagnostics::ask_scroll(ui.ctx());
        ui.scroll_to_rect(widget.rect.expand(AIR), None);
    }
}

/// The size of a dialog's close cross: a header that lays out its own title keeps clear of it
/// (BUG-114).
pub const CROSS: f32 = 20.0;

/// «×» of a dialog or a closable notice: the muted close icon at `size`, «Close» on hover and
/// for a screen reader.
pub fn close_cross(ui: &mut Ui, a: &mut Assets, size: f32) -> Response {
    let p = a.p;
    let img = a.icons.icon("close", p.muted, &p, size);
    let cross = ui
        .add(egui::Button::image(img).frame(false))
        .on_hover_text(crate::i18n::t(tr!("Close")).into_owned());
    // A picture alone says nothing to a screen reader (BUG-41).
    named(&cross, egui::WidgetType::Button, true, tr!("Close"));
    cross
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
    /// Not shown last frame: opened now.
    opened: bool,
}

impl Place {
    /// The dialog's place this frame: as last frame if it was open then, in the same window.
    fn of(ctx: &egui::Context, id: egui::Id) -> Self {
        let pass = ctx.cumulative_pass_nr();
        let window = ctx.content_rect();
        let last = ctx
            .data(|d| d.get_temp::<Place>(id.with("place")))
            .filter(|p| p.pass + 1 >= pass);
        let opened = last.is_none();
        last.filter(|p| p.window == window)
            .map(|p| Place { opened, ..p })
            .unwrap_or(Place {
                top: None,
                frames: 0,
                window,
                pass,
                opened,
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
