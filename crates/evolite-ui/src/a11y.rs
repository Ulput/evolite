//! Keyboard and screen reader plumbing the app and the installer share: a Tab stop of its own for
//! a row of widgets (BUG-40), a name for a text field (BUG-41) and the keyboard of a dialog: where
//! its focus starts and what Enter does in its fields (the designer's rule b′).

use egui::{Event, Id, Key, Response, Sense, Ui};
use std::time::{Duration, Instant};

/// Clicks and a Tab stop for a whole row drawn with `ui.horizontal(..)` (a text action, a
/// checkbox and its words), under an id of its own. Not `row.interact(..)`: egui registers the
/// row's `Ui` again with its hover-only sense under that id and takes the focus back from it, so
/// Tab lost the focus there (BUG-40, as the settings rows in BUG-18). Registered after the row's
/// widgets, it is on top of them: a click on the words is the row's.
pub fn row_click(ui: &Ui, row: &Response, salt: &str) -> Response {
    ui.interact(row.rect, row.id.with(salt), Sense::click())
}

fn next_field_name_id() -> Id {
    Id::new("evolite-next-field-name")
}

/// The name a screen reader says for the next text field drawn in this frame: its label above
/// (`components::field_label` calls this), or what a field without one is for. Without it Tab
/// stops on a field that is only «edit text» (BUG-41).
pub fn name_next_field(ui: &Ui, name: &str) {
    let pass = ui.ctx().cumulative_pass_nr();
    ui.data_mut(|d| d.insert_temp(next_field_name_id(), (pass, name.to_owned())));
}

/// Gives the text field `resp` the name [`name_next_field`] left for it in this frame, if any.
pub fn take_field_name(ui: &Ui, resp: &Response) {
    let pass = ui.ctx().cumulative_pass_nr();
    let name = ui.data_mut(|d| d.remove_temp::<(u64, String)>(next_field_name_id()));
    if let Some((at, name)) = name
        && at == pass
    {
        name_field(ui, resp, &name);
    }
}

/// Names a text field for screen readers, in the interface language. Not
/// `components::named`: the text edit's role (a password field) and value are its own; only
/// the name is added.
pub fn name_field(ui: &Ui, resp: &Response, name: &str) {
    let name = crate::i18n::spoken(name);
    ui.ctx()
        .accesskit_node_builder(resp.id, |node| node.set_label(name));
}

/// The keyboard of the dialog being drawn (the designer's rule b′, 2026-11-06): where the focus
/// starts when it opens and what Enter does in its fields. `components::dialog` opens and closes
/// the scope ([`dialog_begin`], [`dialog_end`]), every text field reports to it
/// ([`field_drawn`]) and its row of buttons asks it ([`dialog_submit`], [`dialog_place`]).
#[derive(Clone, Copy)]
struct Keys {
    /// The pass it was last drawn in.
    pass: u64,
    /// When it opened (a clock of its own, not egui's: a fresh key press is measured from it).
    opened: Instant,
    /// It has opened and its focus is not placed yet (in its first field, else on a button of
    /// its row).
    unplaced: bool,
    /// The pointer is pressed or let go this frame: egui takes a focus placed now away again in
    /// the frame's next pass (a click elsewhere), so a dialog opened by a click places it a
    /// frame later.
    pointer: bool,
    /// Its action cannot be undone: a Danger button or a row marked so (this frame once the
    /// row is drawn, before that the last frame's).
    destructive: bool,
    /// Enter was pressed in this field of a destructive dialog: the next field takes the focus,
    /// and with none after it the focus stays where it was.
    after: Option<Id>,
    /// Enter was pressed in a field of an ordinary dialog: its row presses the primary.
    submit: bool,
}

fn current_dialog_id() -> Id {
    Id::new("evolite-dialog-keys")
}

/// The keys of the dialog being drawn, changed by `f`; `None` outside a dialog.
fn with_keys<R>(ctx: &egui::Context, f: impl FnOnce(&mut Keys) -> R) -> Option<R> {
    let key = ctx.data(|d| d.get_temp::<Option<Id>>(current_dialog_id()))??;
    let mut keys = ctx.data(|d| d.get_temp::<Keys>(key))?;
    let out = f(&mut keys);
    ctx.data_mut(|d| d.insert_temp(key, keys));
    Some(out)
}

/// The dialog `dialog` is drawn from here to [`dialog_end`] (which gets what this returns).
pub fn dialog_begin(ctx: &egui::Context, dialog: Id) -> Option<Id> {
    let pass = ctx.cumulative_pass_nr();
    let key = dialog.with("keys");
    let last = ctx.data(|d| d.get_temp::<Keys>(key));
    let opening = last.is_none_or(|k| k.pass + 1 < pass);
    let pointer = ctx.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
    let keys = Keys {
        pass,
        opened: last
            .filter(|_| !opening)
            .map_or_else(Instant::now, |k| k.opened),
        unplaced: opening || last.is_some_and(|k| k.unplaced),
        pointer,
        destructive: !opening && last.is_some_and(|k| k.destructive),
        after: None,
        submit: false,
    };
    ctx.data_mut(|d| {
        d.insert_temp(key, keys);
        let outer = d.get_temp::<Option<Id>>(current_dialog_id()).flatten();
        d.insert_temp(current_dialog_id(), Some(key));
        outer
    })
}

/// The end of the dialog [`dialog_begin`] started; `outer`: what it returned.
pub fn dialog_end(ctx: &egui::Context, outer: Option<Id>) {
    // Enter in the last field of a destructive dialog does nothing: the focus stays in it.
    if let Some(Some(last)) = with_keys(ctx, |k| k.after.take()) {
        ctx.memory_mut(|m| m.request_focus(last));
    }
    ctx.data_mut(|d| d.insert_temp(current_dialog_id(), outer));
}

/// Whether a dialog is being drawn.
pub fn in_dialog(ctx: &egui::Context) -> bool {
    with_keys(ctx, |_| ()).is_some()
}

/// A text field `resp` was drawn. In a dialog that has just opened, the first one takes the
/// focus. Enter in it presses the primary button of an ordinary dialog ([`dialog_submit`]); in a
/// destructive one it never presses anything and goes on to the next field.
pub fn field_drawn(ui: &Ui, resp: &Response) {
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let focus = with_keys(ui.ctx(), |k| {
        let mut focus = false;
        if k.unplaced && !k.pointer {
            k.unplaced = false;
            focus = true;
        }
        if k.after.take().is_some() {
            focus = true;
        }
        if enter {
            if k.destructive {
                k.after = Some(resp.id);
            } else {
                k.submit = true;
            }
        }
        focus
    });
    if focus == Some(true) {
        ui.memory_mut(|m| m.request_focus(resp.id));
    }
}

/// Takes the Enter pressed in a field of the dialog this frame, for its primary button; never
/// in a destructive dialog.
pub fn dialog_submit(ctx: &egui::Context) -> bool {
    with_keys(ctx, |k| !k.destructive && std::mem::take(&mut k.submit)) == Some(true)
}

/// Says whether the dialog's action cannot be undone, as its row of buttons knows it.
pub fn dialog_destructive(ctx: &egui::Context, destructive: bool) {
    with_keys(ctx, |k| k.destructive = destructive);
}

/// Whether the dialog's action is known to be one that cannot be undone (from its row of
/// buttons, this frame or the last).
pub fn dialog_is_destructive(ctx: &egui::Context) -> bool {
    with_keys(ctx, |k| k.destructive) == Some(true)
}

/// How long a dialog that spends is open before Enter or Space presses its primary (the
/// designer, 2026-11-06): a second Enter right after the one that opened «Check the payment»
/// sends nothing; one after the amount and the address are read does.
pub const SPEND_KEYS_AFTER: Duration = Duration::from_millis(500);

/// Whether Enter or Space is pressed this frame (a fresh press or the key's auto-repeat).
pub fn confirm_key_pressed(ctx: &egui::Context) -> bool {
    ctx.input(|i| {
        i.events.iter().any(|e| {
            matches!(
                e,
                Event::Key {
                    key: Key::Enter | Key::Space,
                    pressed: true,
                    ..
                }
            )
        })
    })
}

/// Whether the Enter or Space of this frame may press the primary of a dialog that spends: a
/// fresh press (not the key's auto-repeat, so not one held since before the dialog opened) made
/// [`SPEND_KEYS_AFTER`] or more after the dialog opened. A click needs no wait.
pub fn spend_key_fresh(ctx: &egui::Context) -> bool {
    let Some(opened) = with_keys(ctx, |k| k.opened) else {
        return false;
    };
    opened.elapsed() >= SPEND_KEYS_AFTER
        && ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    Event::Key {
                        key: Key::Enter | Key::Space,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            })
        })
}

/// Takes this frame's Enter and Space presses away, so the focused button does not take them
/// for a click.
pub fn swallow_confirm_keys(ctx: &egui::Context) {
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            !matches!(
                e,
                Event::Key {
                    key: Key::Enter | Key::Space,
                    pressed: true,
                    ..
                }
            )
        });
    });
}

/// The dialog's clock for a fresh key starts again now: its primary that spends has only now
/// taken the focus (enabled late: the payment's address on its way), and what it does has only
/// now been seen (review r2, should 1).
pub fn dialog_restart_clock(ctx: &egui::Context) {
    with_keys(ctx, |k| k.opened = Instant::now());
}

/// Whether the opening focus is still to be placed (no field took it); placed from now on.
pub fn dialog_place(ctx: &egui::Context) -> bool {
    with_keys(ctx, |k| {
        let place = k.unplaced && !k.pointer;
        k.unplaced &= !place;
        place
    }) == Some(true)
}
