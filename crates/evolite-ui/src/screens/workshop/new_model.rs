//! W5 «New model» (FEAT-0038 UC-5, UC-5a, R-21, R-22), its review and the issue's steps (W6).
//!
//! The picture is chosen from a file and never decoded here: the core takes its metadata out,
//! weighs it against the room of this very write (it depends on the name and description) and
//! keeps it to be drawn by the sandboxed renderer. Over the room, a raster gets «Make a smaller
//! copy» (the renderer makes it: JPEG, or PNG with a palette); the copy is shown beside the
//! original, and only «Use» puts it in the model. The person's file is never changed and
//! nothing is written to the disk.

use std::sync::mpsc::{Receiver, TryRecvError, channel};

use super::*;
use crate::components::spend_flow::SpendFlow;
use crate::components::{
    FieldKind, Part, dialog_with_summary, error_notice, field_label, kv_row_fit, text_field,
};
use evolite_api::series::{
    DOES_NOT_FIT, FittedPicture, NewModel, OUT_OF_TIME, PictureCheck, SeriesPlan, SeriesRefusal,
};
use evolite_api::{Duffs, Spend};

const NAME_MAX: usize = 64;
const DESCRIPTION_MAX: usize = 280;
/// The most read of a file: the renderer takes no more (16 MB).
#[cfg(not(test))]
const FILE_MAX: u64 = 16 * 1024 * 1024;
/// How long the form rests before the core checks it again.
const SETTLE_SECS: f64 = 0.6;

/// A file chosen: its name and bytes, in memory only.
#[derive(Clone)]
struct Chosen {
    name: String,
    bytes: Vec<u8>,
}

/// What the review showed when it was signed.
#[derive(Default, Debug, PartialEq, Clone)]
pub struct Signed {
    name: String,
    mint: u32,
}

/// The form of W5, its check, the copy, the review and the steps.
#[derive(Default)]
pub struct ModelForm {
    contract: String,
    name: String,
    description: String,
    supply: String,
    mint: String,
    file: Option<Chosen>,
    /// A file being chosen (the dialog runs off the frame).
    picking: Option<Receiver<Option<Chosen>>>,
    /// The file's type did not match its name: what it is, and the name's extension.
    mismatch: Option<(&'static str, String)>,
    /// The original file's check (R-22: «removed» is said for a copy of it too).
    original: Option<PictureCheck>,
    /// «Make a smaller copy»: asked, its answer, and whether the copy is in use.
    pub(crate) fitting: bool,
    fitted: Option<Result<FittedPicture, CoreError>>,
    using_copy: bool,
    /// The plan asked for and the last answer, with what it was for.
    asked: Option<(NewModel, u32)>,
    changed_at: f64,
    plan: Option<((NewModel, u32), Result<SeriesPlan, CoreError>)>,
    /// BUG-266: when a «not synced» plan was first seen; it is asked again.
    not_synced: evolite_api::retry::Retry,
    /// W7 r2: the sync's stage when the form last looked.
    sync_seen: Option<evolite_api::SyncStage>,
    review: bool,
    pub(crate) flow: SpendFlow<Signed>,
}

impl ModelForm {
    pub fn new(contract: &str) -> Self {
        Self {
            contract: contract.to_owned(),
            supply: "100".into(),
            mint: "10".into(),
            ..Self::default()
        }
    }

    /// Something was signed: the steps are shown.
    pub fn signed(&self) -> bool {
        self.flow.signed().is_some()
    }

    pub fn forget(&mut self) {
        self.flow.forget();
        *self = Self::default();
    }

    fn supply(&self) -> Option<u32> {
        self.supply.trim().parse().ok().filter(|n| *n > 0)
    }

    fn mint(&self) -> Option<u32> {
        self.mint.trim().parse().ok()
    }

    /// The bytes the model carries now: the copy once «Use» was pressed, else the file.
    fn image(&self) -> Option<Vec<u8>> {
        match (&self.fitted, self.using_copy, &self.file) {
            (Some(Ok(copy)), true, _) => Some(copy.image.clone()),
            (_, _, Some(file)) => Some(file.bytes.clone()),
            _ => None,
        }
    }

    /// The model the form makes now, with how many are issued at once.
    fn draft(&self) -> Option<(NewModel, u32)> {
        Some((
            NewModel {
                name: self.name.trim().to_owned(),
                description: self.description.trim().to_owned(),
                supply: self.supply()?,
                image: self.image()?,
            },
            self.mint()?,
        ))
    }

    /// The plan of what the form shows now, if it came.
    fn planned(&self) -> Option<&Result<SeriesPlan, CoreError>> {
        let draft = self.draft()?;
        self.plan
            .as_ref()
            .filter(|(asked, _)| *asked == draft)
            .map(|(_, r)| r)
    }

    /// [`evolite_api::Event::ModelPlanned`].
    pub fn on_planned(&mut self, result: &Result<SeriesPlan, CoreError>) {
        let Some(asked) = self.asked.clone() else {
            return;
        };
        if let Ok(plan) = result
            && let Some(check) = &plan.picture
            && !self.using_copy
        {
            self.original = Some(check.clone());
        }
        // R-21: the room shrank under the copy in use (a longer name): the copy goes, the offer
        // comes again — nothing is made smaller unseen.
        if self.using_copy
            && let Ok(plan) = result
            && matches!(plan.refusal, Some(SeriesRefusal::ImageTooBig(_)))
        {
            self.using_copy = false;
            self.fitted = None;
            self.asked = None;
            return;
        }
        self.plan = Some((asked, result.clone()));
    }

    /// [`evolite_api::Event::PictureFitted`].
    pub fn on_fitted(&mut self, result: &Result<FittedPicture, CoreError>) {
        if self.fitting {
            self.fitting = false;
            self.fitted = Some(result.clone());
        }
    }

    pub fn on_done(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) -> bool {
        let mine = self.flow.on_done(ask, result);
        if mine && self.flow.password.is_wrong() {
            self.review = true;
        }
        mine
    }

    /// A file chosen (or none): the copy and the checks of the last one go.
    fn take_file(&mut self, chosen: Chosen) {
        self.mismatch = mismatch(&chosen.name, &chosen.bytes);
        self.file = Some(chosen);
        self.fitted = None;
        self.fitting = false;
        self.using_copy = false;
        self.original = None;
        self.plan = None;
        self.asked = None;
    }

    /// `--screen workshop-model` and `-copy`: a large photo chosen as `file`, the model named;
    /// with `copy`, its smaller copy asked for (the command is the caller's to send).
    /// `--screen workshop-model-review`: a small picture that fits, «Check the model» open.
    #[cfg(feature = "dev-tools")]
    pub fn preset_review(&mut self, name: &str) {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><rect width="4" height="4" fill="orange"/></svg>"#;
        let _ = self.preset(name, "pin.svg", svg.to_vec(), false);
        self.review = true;
    }

    #[cfg(feature = "dev-tools")]
    pub fn preset(
        &mut self,
        name: &str,
        file: &str,
        bytes: Vec<u8>,
        copy: bool,
    ) -> Option<Command> {
        self.name = name.into();
        self.take_file(Chosen {
            name: file.into(),
            bytes: bytes.clone(),
        });
        self.fitting = copy;
        // Asked of the core at once (the sweeps wait only for a quiet screen).
        self.changed_at = -SETTLE_SECS;
        copy.then_some(Command::FitPicture {
            image: bytes,
            room: 19_000,
        })
    }
}

/// The type a file's first bytes say, as the profile names it.
fn sniffed(bytes: &[u8]) -> Option<&'static str> {
    let starts = |p: &[u8]| bytes.starts_with(p);
    if starts(b"\x89PNG") {
        Some("PNG")
    } else if starts(&[0xFF, 0xD8, 0xFF]) {
        Some("JPEG")
    } else if starts(b"GIF8") {
        Some("GIF")
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("WebP")
    } else if starts(b"glTF") {
        Some("GLB")
    } else if std::str::from_utf8(&bytes[..bytes.len().min(4096)]).is_ok_and(|t| t.contains("<svg"))
    {
        Some("SVG")
    } else {
        None
    }
}

/// A name whose extension says another type than the bytes (W5: «This is not PNG, though the
/// file is named .png»): what the name claims, and the extension.
fn mismatch(name: &str, bytes: &[u8]) -> Option<(&'static str, String)> {
    let ext = name.rsplit_once('.')?.1.to_lowercase();
    let claimed = match ext.as_str() {
        "png" => "PNG",
        "jpg" | "jpeg" => "JPEG",
        "gif" => "GIF",
        "webp" => "WebP",
        "svg" => "SVG",
        "glb" => "GLB",
        _ => return None,
    };
    (sniffed(bytes) != Some(claimed)).then_some((claimed, ext))
}

/// Opens the system's file dialog off the frame; the file is read whole, up to [`FILE_MAX`].
fn pick(ctx: &egui::Context) -> Receiver<Option<Chosen>> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    // BUG-344: the window the dialog belongs to, taken on the frame (it is in front now): without
    // it Windows opened the dialog behind evolite or off the screen.
    let parent = crate::qr_scan::Parent::now();
    let _ = std::thread::Builder::new()
        .name("workshop-picture".into())
        .spawn(move || {
            let _ = tx.send(choose(parent));
            ctx.request_repaint();
        });
    rx
}

#[cfg(test)]
pub static TEST_PICK: std::sync::Mutex<Option<(String, Vec<u8>)>> = std::sync::Mutex::new(None);

fn choose(parent: crate::qr_scan::Parent) -> Option<Chosen> {
    #[cfg(test)]
    {
        let _ = parent;
        let (name, bytes) = TEST_PICK.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        Some(Chosen { name, bytes })
    }
    #[cfg(not(test))]
    {
        let dialog = rfd::FileDialog::new()
            .set_title(crate::i18n::t(tr!("Picture")).into_owned())
            .add_filter(
                "PNG, JPEG, GIF, WebP, SVG, GLB",
                &["png", "jpg", "jpeg", "gif", "webp", "svg", "glb"],
            );
        #[cfg(windows)]
        let dialog = dialog.set_parent(&parent);
        #[cfg(not(windows))]
        let _ = parent;
        let path = dialog.pick_file()?;
        let file = std::fs::File::open(&path).ok()?;
        let mut bytes = Vec::new();
        use std::io::Read;
        file.take(FILE_MAX + 1).read_to_end(&mut bytes).ok()?;
        let name = path.file_name()?.to_string_lossy().into_owned();
        Some(Chosen { name, bytes })
    }
}

/// A refusal of the model as W5 says it.
fn refusal_words(r: &SeriesRefusal, room: Option<usize>, who: &str) -> String {
    let room_text = room.map_or("—".to_owned(), |r| crate::text::bytes(r as u64));
    match r {
        SeriesRefusal::NameLength(_) => {
            format!(tr!("The name takes 1 to {} characters."), NAME_MAX)
        }
        SeriesRefusal::DescriptionLength(_) => {
            format!(tr!("The description takes up to {} characters."), DESCRIPTION_MAX)
        }
        SeriesRefusal::Link => {
            tr!("No links: evolite does not show models whose text has a site address.").into()
        }
        SeriesRefusal::Supply => tr!("Issue now takes from 0 to the supply.").into(),
        SeriesRefusal::ImageType => tr!("This file is not a picture evolite can write.").into(),
        SeriesRefusal::ImageUnsafe(_) => {
            tr!("This SVG could run code or reach a site when shown: it cannot be written.").into()
        }
        SeriesRefusal::ImageUnreadable => tr!(
            "Could not check the picture: the file is damaged or built in an unusual way. Save it again in an image editor."
        )
        .into(),
        SeriesRefusal::ImageTooBig(size) => format!(
            tr!("The picture is {}, but the record has room for {}."),
            crate::text::bytes(*size as u64),
            room_text
        ),
        SeriesRefusal::NoKey => tr!(
            "This identity has no CRITICAL key for writing contracts. Add one in Identity › Keys."
        )
        .into(),
        SeriesRefusal::NotEnoughCredits { have, need } => format!(
            tr!("Not enough credits: about {need} is needed, {name} has {have}."),
            need = crate::format::amount(*need),
            name = crate::i18n::own(who),
            have = crate::format::amount(*have)
        ),
        SeriesRefusal::SeriesRunning => tr!("A series is running — wait for it or stop it").into(),
        _ => tr!("This model cannot be written.").into(),
    }
}

/// W5, or its steps once signed.
pub(super) fn page(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut WorkshopState,
    art: &mut crate::art::ArtCache,
) {
    let wallet = cx.wallet().clone();
    let Some(identity) = wallet.identity.as_ref().map(|i| i.id.clone()) else {
        state.page = Page::Home;
        return;
    };
    let contract = state.model.contract.clone();
    ui.spacing_mut().item_spacing.y = 16.0;
    if crate::components::back_link(ui, a, tr!("Collection")).clicked()
        && !state.model.flow.waiting()
    {
        state.model.forget();
        state.page = Page::Collection(contract);
        return;
    }
    if state.model.signed() {
        issuing(ui, a, cx, state, &contract);
        return;
    }
    page_header(ui, a, tr!("New model"), None, |_, _| {});
    let form = &mut state.model;
    let held_credits = form.flow.credits(crate::components::Credits::of(&wallet));
    // A file chosen in the dialog.
    if let Some(rx) = &form.picking {
        match rx.try_recv() {
            Ok(Some(chosen)) => {
                form.picking = None;
                form.take_file(chosen);
            }
            Ok(None) | Err(TryRecvError::Disconnected) => form.picking = None,
            Err(TryRecvError::Empty) => {}
        }
    }
    let now = ui.input(|i| i.time);
    // BUG-266: «wait for the sync» is not the last word: asked again a moment later.
    let answer = super::not_synced(form.planned());
    let moved = super::sync_moved(&mut form.sync_seen, cx.snap.sync.stage);
    if super::retry_not_synced(ui, now, answer, moved, &mut form.not_synced) {
        form.asked = None;
        form.plan = None;
    }
    // The core checks the model once the form rests on something it has not checked.
    if let Some(draft) = form.draft().filter(|_| form.mismatch.is_none())
        && form.asked.as_ref() != Some(&draft)
    {
        if now - form.changed_at >= SETTLE_SECS {
            form.asked = Some(draft.clone());
            cx.send(Command::PlanModel {
                identity: identity.clone(),
                contract: contract.clone(),
                model: draft.0,
                mint: draft.1,
            });
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(SETTLE_SECS));
        }
    }
    let who = super::super::tokens::shown_name(&wallet);
    let planned = form.planned().cloned();
    let plan = planned.as_ref().and_then(|p| p.as_ref().ok()).cloned();
    let check = plan.as_ref().and_then(|p| p.picture.clone());
    // Why «Check» waits.
    let why: Option<String> = if form.file.is_none() {
        Some(String::new())
    } else if let Some((claimed, ext)) = &form.mismatch {
        Some(format!(
            tr!("This is not {}, though the file is named .{}."),
            claimed, ext
        ))
    } else if form.supply().is_none() || form.mint().is_none() {
        Some(tr!("Issue now takes from 0 to the supply.").into())
    } else {
        match &planned {
            None => Some(String::new()),
            Some(Err(e)) => Some(crate::text::error_text(e)),
            Some(Ok(plan)) => plan
                .refusal
                .as_ref()
                .map(|r| refusal_words(r, check.as_ref().map(|c| c.room), &who)),
        }
    };
    let wide = ui.available_width() >= 760.0;
    let mut pressed_check = false;
    let mut cancel = false;
    let mut left = |ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut ModelForm| {
        picture_card(
            ui,
            a,
            cx,
            art,
            form,
            check.as_ref(),
            plan.as_ref(),
            why.as_deref(),
        );
        fields(ui, a, form, now);
    };
    // BUG-272: a model to plan, not planned yet.
    let counting = form.draft().is_some() && form.mismatch.is_none() && planned.is_none();
    let mut right = |ui: &mut Ui, a: &mut Assets, form: &ModelForm| {
        summary(
            ui,
            a,
            &who,
            held_credits,
            plan.as_ref(),
            counting,
            form.mint().unwrap_or(0),
            why.as_deref(),
            &mut pressed_check,
            &mut cancel,
        );
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            let total = ui.available_width();
            let side = (total * 0.36).clamp(300.0, 380.0);
            ui.vertical(|ui| {
                ui.set_width(total - side - 20.0);
                ui.spacing_mut().item_spacing.y = 16.0;
                left(ui, a, cx, form);
            });
            ui.vertical(|ui| {
                ui.set_width(side);
                right(ui, a, form);
            });
        });
    } else {
        left(ui, a, cx, form);
        right(ui, a, form);
    }
    if cancel {
        state.model.forget();
        state.page = Page::Collection(contract);
        return;
    }
    if pressed_check && why.is_none() {
        state.model.review = true;
    }
    if state.model.review
        && let (Some(plan), Some((model, mint))) = (plan, state.model.draft())
    {
        review(
            ui,
            a,
            cx,
            &mut state.model,
            &identity,
            &contract,
            model,
            mint,
            &plan,
        );
    }
}

/// The picture: the file, its preview, its weight on the room's scale, what was taken out of
/// it, and «Make a smaller copy» when it is over the room (R-21).
#[allow(clippy::too_many_arguments)]
fn picture_card(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    art: &mut crate::art::ArtCache,
    form: &mut ModelForm,
    check: Option<&PictureCheck>,
    plan: Option<&SeriesPlan>,
    why: Option<&str>,
) {
    let p = a.p;
    let width = ui.available_width();
    let mut choose = false;
    crate::components::card(&p, egui::Margin::same(24)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 24));
        ui.spacing_mut().item_spacing.y = 8.0;
        field_label(ui, &p, tr!("Picture"));
        let Some(file) = form.file.clone() else {
            super::super::tokens::para(
                ui,
                tr!(
                    "PNG, JPEG, GIF, WebP, SVG or GLB. Up to 5 KB as one field, up to about 19 KB in parts. The picture is written to the network whole — it cannot be swapped."
                ),
                12.0,
                p.muted,
                None,
            );
            choose = button_enabled(
                ui,
                a,
                tr!("Choose a file"),
                ButtonKind::Secondary,
                None,
                form.picking.is_none(),
            )
            .clicked();
            return;
        };
        let copy = match (&form.fitted, form.using_copy) {
            (Some(Ok(copy)), true) => Some(copy.clone()),
            _ => None,
        };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let (rect, _) = ui.allocate_exact_size(vec2(120.0, 120.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(10), p.raised);
            let shown = copy
                .as_ref()
                .map(|c| c.art.clone())
                .or(check.map(|c| c.art.clone()))
                .filter(|a| !a.is_empty());
            if let Some(key) = shown {
                let mut send = |c: Command| cx.send(c);
                art.paint(ui, &p, &key, rect, crate::art::Consent::Own, &mut send);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(txt(&crate::i18n::own(&file.name), 13.0, Weight::Medium, p.text));
                let from = crate::text::bytes(file.bytes.len() as u64);
                if let Some(check) = check {
                    let to = crate::text::bytes(check.size as u64);
                    let line = if check.size == file.bytes.len() {
                        from.clone()
                    } else {
                        // R-22: smaller for its metadata taken out, or packed — the lines below
                        // say which; not «after compression» (the analyst, W5 strings).
                        format!(tr!("{} · written as {}"), from, to)
                    };
                    ui.label(txt(&line, 12.0, Weight::Regular, p.muted));
                    // The scale «12 of 19 KB · 3 parts».
                    let fraction = (check.size as f32 / check.room.max(1) as f32).min(1.0);
                    crate::components::progress_bar(ui, &p, fraction);
                    let parts = plan
                        .and_then(|p| p.model.as_ref())
                        .map_or(0, |m| m.parts as usize);
                    let mut scale = format!(
                        tr!("{} of {}"),
                        crate::text::bytes(check.size as u64),
                        crate::text::bytes(check.room as u64)
                    );
                    if parts > 1 {
                        scale = format!(
                            "{scale} · {}",
                            crate::text::plural(parts as u32, tr!("part"), tr!("parts"))
                        );
                    }
                    ui.label(txt(&scale, 12.0, Weight::Regular, p.muted));
                    if parts > 1 {
                        ui.label(txt(
                            &format!(
                                tr!("The picture is written in {} — still one record, at a higher price."),
                                crate::text::plural(parts as u32, tr!("part"), tr!("parts"))
                            ),
                            12.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    }
                } else {
                    ui.label(txt(&from, 12.0, Weight::Regular, p.muted));
                }
                if button(ui, a, tr!("Choose another file"), ButtonKind::Ghost, None).clicked() {
                    choose = true;
                }
            });
        });
        // R-22: what was taken out, said against the original file.
        let removed = check.is_some_and(|c| c.removed)
            || (copy.is_some() && form.original.as_ref().is_some_and(|c| c.removed));
        if removed {
            super::super::tokens::para(
                ui,
                tr!("Details stored in the file, such as the location and the camera, were removed"),
                12.0,
                p.muted,
                None,
            );
        }
        if check.is_some_and(|c| c.glb) {
            super::super::tokens::para(
                ui,
                tr!(
                    "Details inside a 3D model, such as the program it was made in, are not removed and are written with it"
                ),
                12.0,
                p.muted,
                None,
            );
        }
        fit_offer(ui, a, cx, art, form, check, plan, why);
    });
    if choose && form.picking.is_none() {
        form.picking = Some(pick(ui.ctx()));
    }
}

/// R-21: over the room — for a raster the offer and «Make a smaller copy», its run and its
/// result with «Use»; for SVG and GLB the honest refusal; a picture that could not be read or
/// does not fit at all, its line.
#[allow(clippy::too_many_arguments)]
fn fit_offer(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    art: &mut crate::art::ArtCache,
    form: &mut ModelForm,
    check: Option<&PictureCheck>,
    plan: Option<&SeriesPlan>,
    why: Option<&str>,
) {
    let p = a.p;
    let too_big = matches!(
        plan.and_then(|p| p.refusal.as_ref()),
        Some(SeriesRefusal::ImageTooBig(_))
    );
    // A picture refusal other than «too big» is said here, under the picture.
    let picture_refusal = plan.and_then(|p| p.refusal.as_ref()).is_some_and(|r| {
        matches!(
            r,
            SeriesRefusal::ImageType
                | SeriesRefusal::ImageUnsafe(_)
                | SeriesRefusal::ImageUnreadable
        )
    });
    if (picture_refusal || form.mismatch.is_some())
        && let Some(why) = why.filter(|w| !w.is_empty())
    {
        super::super::tokens::para(ui, why, 12.0, p.danger, None);
        return;
    }
    // The copy, made and not yet used: both pictures' sizes, «Use» or another file.
    if let (Some(Ok(copy)), false) = (&form.fitted, form.using_copy) {
        let copy = copy.clone();
        // The copy as it will be written, beside the original above (UC-5a, step 3).
        let (rect, _) = ui.allocate_exact_size(vec2(120.0, 120.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(10), p.raised);
        let mut send = |c: Command| cx.send(c);
        art.paint(ui, &p, &copy.art, rect, crate::art::Consent::Own, &mut send);
        ui.label(txt(
            &format!(
                tr!("Original file: {}×{}, {}"),
                copy.source_width,
                copy.source_height,
                crate::text::bytes(form.file.as_ref().map_or(0, |f| f.bytes.len()) as u64)
            ),
            12.0,
            Weight::Regular,
            p.muted,
        ));
        ui.label(txt(
            &format!(
                tr!("Reduced: {}×{}, {}"),
                copy.width,
                copy.height,
                crate::text::bytes(copy.image.len() as u64)
            ),
            12.0,
            Weight::Regular,
            p.text,
        ));
        if button(ui, a, tr!("Use"), ButtonKind::Primary, None).clicked() {
            form.using_copy = true;
        }
        return;
    }
    if !too_big {
        return;
    }
    let Some(check) = check else {
        return;
    };
    let size = crate::text::bytes(check.size as u64);
    let room = crate::text::bytes(check.room as u64);
    if !check.can_fit {
        let kind = form
            .file
            .as_ref()
            .and_then(|f| sniffed(&f.bytes))
            .unwrap_or("SVG");
        super::super::tokens::para(
            ui,
            &format!(
                tr!(
                    "The {} file is {}, but the record has room for {}. evolite does not shrink SVG or GLB: make the file smaller in an editor."
                ),
                kind, size, room
            ),
            12.0,
            p.danger,
            None,
        );
        return;
    }
    match &form.fitted {
        Some(Err(CoreError::Failed(why))) if why == DOES_NOT_FIT => {
            super::super::tokens::para(
                ui,
                tr!(
                    "This picture does not fit into the record even at a small size. Choose another one or simplify this one."
                ),
                12.0,
                p.danger,
                None,
            );
            return;
        }
        // Batch 65: the time ran out, not the room — the picture may fit on another try, so
        // «Make a smaller copy» stays below.
        Some(Err(CoreError::Failed(why))) if why == OUT_OF_TIME => {
            super::super::tokens::para(
                ui,
                tr!(
                    "Making a smaller copy took too long this time. Try again; if it keeps happening, choose a smaller picture."
                ),
                12.0,
                p.danger,
                None,
            );
        }
        Some(Err(_)) => {
            super::super::tokens::para(ui, tr!("Could not read the picture"), 12.0, p.danger, None);
            return;
        }
        _ => {}
    }
    super::super::tokens::para(
        ui,
        &format!(
            tr!(
                "The picture is {}, but the record has room for {}. evolite can make a smaller copy that fits; the file itself stays as it is."
            ),
            size, room
        ),
        12.0,
        p.text,
        None,
    );
    let animated = form
        .file
        .as_ref()
        .is_some_and(|f| sniffed(&f.bytes).is_some_and(|k| k == "GIF" || k == "WebP"));
    if animated {
        super::super::tokens::para(
            ui,
            tr!("Only the first frame goes into the copy: it will not move."),
            12.0,
            p.muted,
            None,
        );
    }
    if form.fitting {
        ui.label(txt(
            tr!("Making a smaller copy…"),
            12.0,
            Weight::Medium,
            p.muted,
        ));
    } else if button(ui, a, tr!("Make a smaller copy"), ButtonKind::Primary, None).clicked()
        && let Some(file) = &form.file
    {
        form.fitting = true;
        // Another try after «took too long»: its words go until the new answer.
        form.fitted = None;
        cx.send(Command::FitPicture {
            image: file.bytes.clone(),
            room: check.room,
        });
    }
}

/// The name, the description, the supply and «Issue now», with their counters.
fn fields(ui: &mut Ui, a: &mut Assets, form: &mut ModelForm, now: f64) {
    let p = a.p;
    let width = ui.available_width();
    let before = (
        form.name.clone(),
        form.description.clone(),
        form.supply.clone(),
        form.mint.clone(),
    );
    crate::components::card(&p, egui::Margin::same(24)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 24));
        ui.spacing_mut().item_spacing.y = 8.0;
        let counted = |ui: &mut Ui, label: &str, n: usize, max: usize| {
            ui.horizontal(|ui| {
                field_label(ui, &p, label);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(txt(&format!("{n} / {max}"), 11.0, Weight::Medium, p.muted));
                });
            });
        };
        counted(ui, tr!("Model name"), form.name.chars().count(), NAME_MAX);
        text_field(
            ui,
            &p,
            "workshop-model-name",
            &mut form.name,
            "",
            FieldKind::Text,
            |_| {},
        );
        counted(
            ui,
            tr!("Description"),
            form.description.chars().count(),
            DESCRIPTION_MAX,
        );
        crate::components::text_area(
            ui,
            &p,
            "workshop-model-description",
            &mut form.description,
            "",
            3,
        );
        super::super::tokens::para(
            ui,
            tr!("No links: evolite does not show models whose text has a site address."),
            12.0,
            p.muted,
            None,
        );
        // What is typed past the schema's bounds is not kept.
        if form.name.chars().count() > NAME_MAX {
            form.name = form.name.chars().take(NAME_MAX).collect();
        }
        if form.description.chars().count() > DESCRIPTION_MAX {
            form.description = form.description.chars().take(DESCRIPTION_MAX).collect();
        }
        ui.add_space(8.0);
        field_label(
            ui,
            &p,
            tr!("Supply · How many copies there can be in all. Cannot be changed."),
        );
        text_field(
            ui,
            &p,
            "workshop-model-supply",
            &mut form.supply,
            "100",
            FieldKind::Text,
            |_| {},
        );
        field_label(ui, &p, tr!("Issue now · The rest — later."));
        text_field(
            ui,
            &p,
            "workshop-model-mint",
            &mut form.mint,
            "0",
            FieldKind::Text,
            |_| {},
        );
    });
    form.supply.retain(|c| c.is_ascii_digit());
    form.mint.retain(|c| c.is_ascii_digit());
    if (
        form.name.clone(),
        form.description.clone(),
        form.supply.clone(),
        form.mint.clone(),
    ) != before
    {
        form.changed_at = now;
    }
}

/// «Summary»: the model, the copies, the total, the credits after, the one-signature line.
#[allow(clippy::too_many_arguments)]
fn summary(
    ui: &mut Ui,
    a: &mut Assets,
    who: &str,
    credits: crate::components::Credits,
    plan: Option<&SeriesPlan>,
    counting: bool,
    mint: u32,
    why: Option<&str>,
    check: &mut bool,
    cancel: &mut bool,
) {
    let p = a.p;
    let width = ui.available_width();
    let value = crate::theme::mono(13.0, Weight::Medium);
    // BUG-259: the estimates carry their margin — «at most».
    let about = |d: Option<Duffs>| super::at_most(d, counting);
    let model_fee = plan.and_then(|p| p.model.as_ref()).map(|m| m.fee);
    // The copies: what the total holds besides the model (the first copy costs more).
    let copies = plan.and_then(|p| Some(p.total.saturating_sub(p.model.as_ref()?.fee)));
    let total = plan.filter(|p| p.model.is_some()).map(|p| p.total);
    crate::components::card(&p, egui::Margin::same(24)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 24));
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.label(txt(tr!("Summary"), 18.0, Weight::SemiBold, p.text));
        kv_row_fit(
            ui,
            &p,
            tr!("Identity"),
            &crate::i18n::own(who),
            value.clone(),
        );
        kv_row_fit(ui, &p, tr!("Model"), &about(model_fee), value.clone());
        // BUG-343: no copies issued now — no row of «0 copies · 0.0000».
        if mint > 0 {
            kv_row_fit(
                ui,
                &p,
                &crate::text::plural(mint, tr!("copy"), tr!("copies")),
                &about(copies),
                value.clone(),
            );
        }
        crate::components::divider(ui, &p);
        kv_row_fit(ui, &p, tr!("Total"), &about(total), value.clone());
        // FEAT-0057 П3 (R-3.1): the one line «Credits: now › after» for the whole series.
        crate::components::credits_after(
            ui,
            a,
            &crate::i18n::own(who),
            credits,
            total.map_or(
                crate::components::Charge::Calculating,
                crate::components::Charge::AtMost,
            ),
        );
        let once = if mint > 0 {
            format!(
                tr!(
                    "You sign once: evolite writes the model and {} one by one — {} on the network."
                ),
                crate::text::plural(mint, tr!("copy"), tr!("copies")),
                crate::text::plural(mint + 1, tr!("record"), tr!("records"))
            )
        } else {
            // BUG-343: the model alone — not «the model and 0 copies».
            tr!("You sign once: evolite writes the model — 1 record on the network.").to_owned()
        };
        super::super::tokens::para(ui, &once, 12.0, p.muted, None);
        crate::components::button_row(ui, |ui| {
            if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                *cancel = true;
            }
            *check = button_enabled(
                ui,
                a,
                tr!("Check"),
                ButtonKind::Primary,
                None,
                why.is_none(),
            )
            .clicked();
        });
        if let Some(why) = why.filter(|w| !w.is_empty()) {
            super::super::tokens::para(ui, why, 12.0, p.danger, None);
        }
    });
}

/// «Check the model»: what is written, the fee, the password, «Sign and issue».
#[allow(clippy::too_many_arguments)]
fn review(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut ModelForm,
    identity: &str,
    contract: &str,
    model: NewModel,
    mint: u32,
    plan: &SeriesPlan,
) {
    let name = model.name.clone();
    let supply = model.supply;
    let op = Spend::CreateModel {
        identity: identity.to_owned(),
        contract: contract.to_owned(),
        model: model.clone(),
        mint,
        fee_max: plan.total,
    };
    // FEAT-0004 FR-7: the large spend's tick from the start, in the summary.
    form.flow.password.watch(Some(&op));
    // BUG-249: what is signed — the model, its copies, the total, that the picture stays — is
    // pinned over the password.
    let closed = dialog_with_summary(
        ui,
        a,
        "workshop-check-model",
        "nft",
        tr!("Check the model"),
        480.0,
        |ui, a, part| {
            let p = a.p;
            let value = crate::theme::mono(13.0, Weight::Medium);
            if part == Part::Summary {
                ui.spacing_mut().item_spacing.y = 10.0;
                kv_row_fit(
                    ui,
                    &p,
                    tr!("Total"),
                    // BUG-343: the margin's estimate, as in the form — «at most».
                    &super::at_most(Some(plan.total), false),
                    value.clone(),
                );
                crate::components::warning(
                    ui,
                    a,
                    tr!("The picture and description cannot change after issue"),
                );
                form.flow.password.summary_tick(ui, &p);
                return false;
            }
            if part == Part::Footer {
                let mut close = false;
                form.flow
                    .password
                    .field(ui, &p, "workshop-model-password", true);
                if let Some(Err(e)) = form
                    .flow
                    .result()
                    .filter(|_| !form.flow.password.is_wrong())
                {
                    error_notice(
                        ui,
                        a,
                        &crate::text::error_text(e),
                        crate::text::error_details(e).as_deref(),
                    );
                }
                let ready = form.flow.password.ready(true) && form.flow.can_act();
                let mut go = false;
                crate::components::button_row(ui, |ui| {
                    if button(ui, a, tr!("Back"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    go = button_enabled(
                        ui,
                        a,
                        tr!("Sign and issue"),
                        ButtonKind::Primary,
                        None,
                        ready,
                    )
                    .clicked();
                });
                if go {
                    let signed = Signed {
                        name: name.clone(),
                        mint,
                    };
                    if let Some(command) = form.flow.command(op.clone(), true, signed) {
                        cx.send(command);
                    }
                }
                return close;
            }
            ui.spacing_mut().item_spacing.y = 10.0;
            kv_row_fit(
                ui,
                &p,
                tr!("Model name"),
                &crate::i18n::own(&name),
                value.clone(),
            );
            kv_row_fit(
                ui,
                &p,
                tr!("Supply"),
                &crate::format::count(supply as u64),
                value.clone(),
            );
            kv_row_fit(
                ui,
                &p,
                tr!("Issue now"),
                &crate::format::count(mint as u64),
                value.clone(),
            );
            false
        },
    );
    if closed && !form.flow.waiting() {
        form.review = false;
        if !form.signed() {
            form.flow.forget();
        }
    }
    if form.signed() && !form.flow.password.is_wrong() {
        form.review = false;
    }
}

/// W6 «Issuing “{model}”»: the model written, the copies counted, done; the window can be
/// left — the series goes on and its plate shows where.
fn issuing(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut WorkshopState, contract: &str) {
    let p = a.p;
    let Some(signed) = state.model.flow.signed().cloned() else {
        return;
    };
    let result = state.model.flow.result().cloned();
    let series = cx
        .wallet()
        .series
        .iter()
        .filter(|s| s.target.contract == contract)
        .max_by_key(|s| s.id)
        .cloned();
    page_header(
        ui,
        a,
        &format!(tr!("Issuing “{}”"), crate::i18n::own(&signed.name)),
        None,
        |_, _| {},
    );
    let width = ui.available_width().min(560.0);
    let mut leave = false;
    crate::components::card(&p, egui::Margin::same(24)).show(ui, |ui| {
        ui.set_width(crate::components::inner(width, 24));
        ui.spacing_mut().item_spacing.y = 12.0;
        let written = series.is_some() || matches!(result, Some(Ok(())));
        let step = |ui: &mut Ui, a: &mut Assets, words: &str, done: Option<bool>| {
            let p = a.p;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (icon, color) = match done {
                    Some(true) => ("check", p.accent),
                    Some(false) => ("warning", p.danger),
                    None => ("pending", p.muted),
                };
                ui.add(a.icons.icon(icon, color, &p, 16.0));
                // BUG-258: wraps beside its icon at the medium window.
                ui.add(crate::components::para(words, 14.0, Weight::Regular, p.text));
            });
        };
        let failed = matches!(result, Some(Err(_)));
        step(
            ui,
            a,
            tr!("Model written"),
            if written {
                Some(true)
            } else if failed {
                Some(false)
            } else {
                None
            },
        );
        if signed.mint > 0 {
            let done = series.as_ref().map_or(0, |s| s.done);
            let words = format!(tr!("Copies · issued {} of {}"), done, signed.mint);
            step(ui, a, &words, (done >= signed.mint).then_some(true));
            crate::components::progress_bar(
                ui,
                &p,
                done as f32 / signed.mint.max(1) as f32,
            );
        }
        step(ui, a, tr!("Done"), matches!(result, Some(Ok(()))).then_some(true));
        super::super::tokens::para(
            ui,
            tr!(
                "Each copy is a separate record on the network, about 2 seconds. You can hide the window: issuing goes on. If the connection drops, the model’s page shows “Continue issuing” — from the same number."
            ),
            12.0,
            p.muted,
            None,
        );
        if let Some(Err(e)) = &result {
            error_notice(
                ui,
                a,
                &crate::text::error_text(e),
                crate::text::error_details(e).as_deref(),
            );
        }
        crate::components::button_row(ui, |ui| {
            let label = if result.is_some() { tr!("Done") } else { tr!("Hide") };
            leave = button(ui, a, label, ButtonKind::Primary, None).clicked();
        });
    });
    if leave {
        state.model.forget();
        state.read_again();
        state.counts.clear();
        state.counts_asked.clear();
        state.page = Page::Collection(contract.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The analyst (W5 strings): every refusal a new model can get says its own reason — none
    /// falls to «This model cannot be written».
    #[test]
    fn every_model_refusal_has_its_own_words() {
        let fallback = refusal_words(&SeriesRefusal::ZeroPrice, Some(19_000), "@me");
        let refusals = [
            SeriesRefusal::NameLength(65),
            SeriesRefusal::DescriptionLength(281),
            SeriesRefusal::Link,
            SeriesRefusal::Supply,
            SeriesRefusal::ImageType,
            SeriesRefusal::ImageUnsafe("script".into()),
            SeriesRefusal::ImageUnreadable,
            SeriesRefusal::ImageTooBig(30_000),
            SeriesRefusal::NoKey,
            SeriesRefusal::NotEnoughCredits { have: 1, need: 2 },
            SeriesRefusal::SeriesRunning,
        ];
        let mut said = std::collections::HashSet::new();
        for r in refusals {
            let words = refusal_words(&r, Some(19_000), "@me");
            assert_ne!(words, fallback, "{r:?}");
            assert!(said.insert(words), "{r:?} says what another does");
        }
    }
}
