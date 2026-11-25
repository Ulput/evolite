//! FEAT-0054: «Add a collection by id» (Advanced, the fallback): the Collection shows the NFTs
//! the wallet's identities hold in a collection the search did not find. The core keeps only a
//! contract of the evolite standard; the list is the wallet's, with «Remove».

use super::SettingsState;
use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_enabled, dialog, field_label, text_field,
};
use crate::context::Ctx;
use crate::theme::Weight;
use egui::Ui;
use evolite_api::{Command, CoreError};

/// The dialog's form, while it is open.
#[derive(Default)]
pub struct CollectionsForm {
    pub open: bool,
    pub id: String,
    /// An «Add» or «Remove» on its way to the core.
    pub waiting: bool,
    /// The core's refusal of the last «Add», said under the field.
    pub error: Option<String>,
}

impl SettingsState {
    /// The core's answer to «Add» or «Remove»: a refusal under the field, an added id clears it.
    pub fn on_collection(&mut self, result: &Result<(), CoreError>) {
        let form = &mut self.collections;
        form.waiting = false;
        match result {
            Ok(()) => {
                form.id.clear();
                form.error = None;
            }
            Err(e) => form.error = Some(crate::text::error_text(e)),
        }
    }
}

/// The dialog, while open.
pub(super) fn collections_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut SettingsState,
) {
    let p = a.p;
    let added = cx.wallet().added_collections.clone();
    let mut send = None;
    let closed = dialog(
        ui,
        a,
        "collections-dialog",
        "nft",
        tr!("Add a collection by id"),
        462.0,
        |ui, a| {
            let form = &mut state.collections;
            ui.add(crate::components::para(
                tr!(
                    "The Collection shows the NFTs of your identities in this collection too. Only NFT collections of the evolite standard."
                ),
                13.0,
                Weight::Regular,
                p.muted,
            ));
            field_label(ui, &p, tr!("Contract id"));
            text_field(
                ui,
                &p,
                "collection-id",
                &mut form.id,
                tr!("44 letters and digits"),
                FieldKind::Text,
                |_| {},
            );
            if let Some(error) = &form.error {
                ui.add(crate::components::para(
                    error,
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            }
            let ready = !form.waiting && !form.id.trim().is_empty();
            if button_enabled(ui, a, tr!("Add"), ButtonKind::Primary, None, ready).clicked()
                && ready
            {
                form.waiting = true;
                form.error = None;
                send = Some(Command::AddCollection {
                    id: form.id.trim().to_owned(),
                });
            }
            if !added.is_empty() {
                ui.add_space(8.0);
                field_label(ui, &p, tr!("Added"));
                for id in &added {
                    ui.horizontal(|ui| {
                        ui.label(crate::components::txt(
                            &crate::i18n::own(&crate::components::fit_id(id, &|s: &str| {
                                s.chars().count() <= 24
                            })),
                            12.0,
                            Weight::Regular,
                            p.text,
                        ))
                        .on_hover_text(id.as_str());
                        if button(ui, a, tr!("Remove"), ButtonKind::Secondary, None).clicked()
                            && !form.waiting
                        {
                            form.waiting = true;
                            send = Some(Command::RemoveCollection { id: id.clone() });
                        }
                    });
                }
            }
            button(ui, a, tr!("Done"), ButtonKind::Secondary, None).clicked()
        },
    );
    if let Some(command) = send {
        cx.send(command);
    }
    if closed {
        state.collections.open = false;
        state.collections.error = None;
    }
}
