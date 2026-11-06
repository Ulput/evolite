//! Keyboard and screen reader plumbing the app and the installer share: a Tab stop of its own for
//! a row of widgets (BUG-40) and a name for a text field (BUG-41).

use egui::{Id, Response, Sense, Ui};

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
