//! BUG-204, BUG-212 (the analyst's «what the person sees»): a banner on top of each screen that
//! stands on one of the wallet's files when that file did not read and was set aside — money and
//! the network are not touched, the file is not deleted, and what can be done: «Try again», «Show
//! in folder», «Start this part over…» (with what comes back from the network) and «Report a
//! problem». No password: none of these spends or shows a key (R-3).

use crate::components::{Assets, ButtonKind, Part, button, close_cross, dialog_with_footer, para};
use crate::context::Ctx;
use crate::theme::Weight;
use egui::{Align, CornerRadius, Frame, Layout, Margin, Ui};
use evolite_api::{Command, CoreError, LocalFile};

/// The banner's title (each file its own: the cases of a name do not join a template).
fn title(file: LocalFile) -> &'static str {
    match file {
        LocalFile::Journal => tr!("Part of the Platform history could not be read"),
        LocalFile::NameRequests => tr!("Your name requests could not be read"),
        LocalFile::ConnectedApps => tr!("The list of connected apps could not be read"),
        LocalFile::ContactAddresses => tr!("Your contacts' payment addresses could not be read"),
        LocalFile::TopUps => tr!("The records of credit top-ups could not be read"),
        LocalFile::AddressBook => tr!("The address book could not be read"),
        LocalFile::ContactResend => {
            tr!("The list of contacts waiting for your request could not be read")
        }
        LocalFile::NftSeries => tr!("Your Workshop series could not be read"),
        LocalFile::AutoBackup => tr!("The backup setting could not be read"),
    }
}

/// The part's short name in «Start {} over?».
fn part_name(file: LocalFile) -> &'static str {
    match file {
        LocalFile::Journal => tr!("the Platform history"),
        LocalFile::NameRequests => tr!("the name requests"),
        LocalFile::ConnectedApps => tr!("the list of connected apps"),
        LocalFile::ContactAddresses => tr!("the contacts' addresses"),
        LocalFile::TopUps => tr!("the credit top-up records"),
        LocalFile::AddressBook => tr!("the address book"),
        LocalFile::ContactResend => tr!("the list of contacts waiting for your request"),
        LocalFile::NftSeries => tr!("the Workshop series"),
        LocalFile::AutoBackup => tr!("the weekly copy's setting"),
    }
}

/// What comes back from the network when the part starts over (checked against the code by the
/// architect, BUG-204).
fn comes_back(file: LocalFile) -> &'static str {
    match file {
        LocalFile::Journal => tr!(
            "Names, requests to contacts, the profile, withdrawals to Core and actions with tokens that keep their history come back from the network. Credit transfers, Platform address operations, key changes, app sign-ins, requests for contested names and exact Platform fees do not."
        ),
        LocalFile::NameRequests => tr!(
            "evolite stops following the votes on your requests. To follow a vote again, check the same name in “Add a name”: evolite finds your request on the network, and you do not pay again."
        ),
        LocalFile::ConnectedApps => tr!(
            "The list starts empty. The keys the apps received stay on the network: they show in the identity's keys without the app's name, and you can disable them there."
        ),
        LocalFile::ContactAddresses => tr!(
            "evolite rebuilds which of your contacts' addresses were already paid from your sent payments."
        ),
        LocalFile::TopUps => {
            tr!("The top-ups stay in History, but not how many credits each one added.")
        }
        LocalFile::AddressBook => {
            tr!("The address book starts empty. Saved addresses stay only in the set-aside file.")
        }
        LocalFile::ContactResend => tr!(
            "evolite forgets which contacts still wait for a new request from you: their marks in Contacts and the button on the keys screen go away. They still cannot pay you until they get one; the next time you replace the DashPay pair, evolite sends it to all of them."
        ),
        LocalFile::NftSeries => tr!(
            "The series' plates and “Continue” go away: what each series already wrote stays on the network and in your collection, and a series cut short is started again for what is left. evolite reads each model's highest serial from the network again, but a highest serial that was deleted may be issued again."
        ),
        // TECH-bug-prevention (а2): until then no weekly copy is written.
        LocalFile::AutoBackup => tr!(
            "The weekly copy stays off until you choose its folder again. The copies already written stay in their folder."
        ),
    }
}

fn collapsed_id(file: LocalFile) -> egui::Id {
    egui::Id::new(("set-aside-collapsed", format!("{file:?}")))
}

fn confirm_id() -> egui::Id {
    egui::Id::new("set-aside-confirm")
}

fn answer_id(file: LocalFile) -> egui::Id {
    egui::Id::new(("set-aside-answer", format!("{file:?}")))
}

/// What the screen asks the app for: «Report a problem» with this subject filled in.
pub fn report_id() -> egui::Id {
    egui::Id::new("set-aside-report")
}

/// The core's answer to «Try again» or «Start over» (`Done { what: DoneWhat::LocalFile }`):
/// a refusal is said under the banner it came from.
pub fn on_done(ctx: &egui::Context, file: Option<LocalFile>, result: &Result<(), CoreError>) {
    let Some(file) = file else {
        return;
    };
    ctx.data_mut(|d| match result {
        Ok(()) => d.remove::<String>(answer_id(file)),
        Err(e) => {
            d.insert_temp(answer_id(file), crate::text::error_text(e));
        }
    });
}

/// The banners of `files` that were set aside and not collapsed in this run, one under the
/// other; and the confirmation of «Start over», when asked.
pub fn banners(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, files: &[LocalFile]) {
    let set_aside = cx.wallet().set_aside.clone();
    for aside in set_aside.iter().filter(|s| files.contains(&s.file)) {
        if ui.data(|d| d.get_temp::<bool>(collapsed_id(aside.file))) == Some(true) {
            continue;
        }
        banner(ui, a, cx, aside);
        ui.add_space(12.0);
    }
    confirm(ui, a, cx);
}

/// The file whose «Start over» is being confirmed, if it is one of these.
fn confirming(ui: &Ui) -> Option<LocalFile> {
    ui.data(|d| d.get_temp::<Option<LocalFile>>(confirm_id()))
        .flatten()
}

fn banner(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, aside: &evolite_api::SetAside) {
    let p = a.p;
    let file = aside.file;
    let path = std::path::PathBuf::from(&aside.path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Frame::new()
        .fill(p.raised)
        .stroke(egui::Stroke::new(1.0, crate::components::attention_color(&p)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add(
                    a.icons
                        .icon("warning", crate::components::attention_color(&p), &p, 18.0),
                );
                // Wrapped in what the icon and the cross leave: a long title (ru, fr, de) never
                // widens the page (BUG-249, A7).
                let room = (ui.available_width() - 16.0 - 8.0).max(80.0);
                ui.allocate_ui(egui::vec2(room, 0.0), |ui| {
                    ui.set_max_width(room);
                    ui.add(para(title(file), 14.0, Weight::SemiBold, p.text));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Collapsed until the next start: it comes back while the file is aside.
                    let cross = close_cross(ui, a, 16.0);
                    crate::components::named(
                        &cross,
                        egui::WidgetType::Button,
                        true,
                        tr!("Hide until restart"),
                    );
                    if cross.clicked() {
                        ui.data_mut(|d| d.insert_temp(collapsed_id(file), true));
                    }
                });
            });
            // The file's name only, never the folder (AC-A6).
            ui.add(para(
                &format!(
                    tr!(
                        "Money and data on the network are not affected. The file is not deleted: evolite set it aside as {}."
                    ),
                    crate::i18n::own(&name)
                ),
                13.0,
                Weight::Regular,
                p.text,
            ));
            // The journal keeps new lines in memory meanwhile (review r1, Should 1); the others
            // keep nothing new (the analyst).
            ui.add(para(
                if file == LocalFile::Journal {
                    tr!(
                        "New records still show here, but are saved only when the file is back or you start this part over. If evolite is closed before that, they are gone from this computer."
                    )
                } else {
                    tr!(
                        "Until the file is back, new records of this part are not saved on this computer."
                    )
                },
                13.0,
                Weight::Regular,
                p.text,
            ));
            ui.add(para(
                tr!(
                    "This happens when the folder was opened by a newer evolite, or another program (an antivirus, a backup tool) held the file."
                ),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            if let Some(answer) = ui.data(|d| d.get_temp::<String>(answer_id(file))) {
                ui.add(para(&answer, 12.0, Weight::Medium, p.danger));
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if button(ui, a, tr!("Try again"), ButtonKind::Secondary, None).clicked() {
                    ui.data_mut(|d| d.insert_temp(crate::screens::set_aside::asked_id(), file));
                    cx.send(Command::TryFileAgain { file });
                }
                if button(ui, a, tr!("Show in folder"), ButtonKind::Ghost, None).clicked() {
                    crate::links::show_in_folder(&path);
                }
                if button(ui, a, tr!("Start this part over…"), ButtonKind::Ghost, None).clicked()
                {
                    ui.data_mut(|d| d.insert_temp(confirm_id(), Some(file)));
                }
                if crate::components::text_action(ui, a, "info", tr!("Report a problem"))
                    .clicked()
                {
                    // The kind of file only: no path, nothing of what it held.
                    let subject = format!(tr!("A file was set aside: {}"), part_name(file));
                    ui.data_mut(|d| d.insert_temp(report_id(), subject));
                }
            });
        });
}

/// Which file the last «Try again» or «Start over» was for: its answer goes under its banner.
pub fn asked_id() -> egui::Id {
    egui::Id::new("set-aside-asked")
}

/// «Start {} over?»: what comes back from the network, that the file aside stays, «Start over»
/// and «Cancel».
fn confirm(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let Some(file) = confirming(ui) else {
        return;
    };
    let mut start = false;
    let title = format!(tr!("Start {} over?"), part_name(file));
    let closed = dialog_with_footer(
        ui,
        a,
        "set-aside-confirm",
        "warning",
        &title,
        440.0,
        |ui, a, part| {
            let p = a.p;
            match part {
                // Not a dialog with a summary (BUG-249): never asked.
                Part::Summary => false,
                Part::Middle => {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    ui.add(para(comes_back(file), 13.0, Weight::Regular, p.text));
                    ui.add(para(
                    tr!(
                        "The set-aside file stays in the folder: nothing is deleted. evolite starts this part with an empty file."
                    ),
                    13.0,
                    Weight::Regular,
                    p.muted,
                ));
                    false
                }
                Part::Footer => {
                    let mut close = false;
                    // A part's records go: the focus starts on «Cancel» (as the dialogs that cannot
                    // be undone).
                    crate::components::button_row_destructive(ui, |ui| {
                        if button(ui, a, tr!("Start over"), ButtonKind::Primary, None).clicked() {
                            start = true;
                            close = true;
                        }
                        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                            close = true;
                        }
                    });
                    close
                }
            }
        },
    );
    if start {
        ui.data_mut(|d| d.insert_temp(asked_id(), file));
        cx.send(Command::StartFileOver { file });
    }
    if closed || start {
        ui.data_mut(|d| d.remove::<Option<LocalFile>>(confirm_id()));
    }
}

/// Overview, over «Latest operations»: one line that leads to History's banner, when the
/// journal was set aside.
pub fn overview_line(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    if !cx
        .wallet()
        .set_aside
        .iter()
        .any(|s| s.file == LocalFile::Journal)
    {
        return;
    }
    if crate::components::text_action(
        ui,
        a,
        "warning",
        tr!("Part of the history could not be read ›"),
    )
    .clicked()
    {
        cx.go(crate::screens::Screen::Activity);
    }
}
