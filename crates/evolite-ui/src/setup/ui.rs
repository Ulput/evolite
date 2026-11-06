//! The installer's and the uninstaller's window (Figma «Installer», I-01…I-05, U-01…U-04): the
//! same palette, fonts and right-to-left mirroring as evolite. Nothing is written before the
//! person presses «Install» (or «Remove»), and wallet data is deleted only after the warning and
//! its own confirmation.

use super::plan::{self, Disk, FoundWallet, Known, Room};
use super::work::{self, Shared};
use super::{payload::Payload, system};
use crate::i18n::{self, Lang};
use crate::icons::{Icons, Logo};
use crate::theme::{self, Palette, Weight, mono, sans};
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Layout, Margin, Panel, Rect, RichText,
    Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What this run of the program is.
#[derive(Clone)]
pub enum Mode {
    /// The installer, carrying the program.
    Install { source: PathBuf, payload: Payload },
    /// The uninstaller, in the program folder.
    Uninstall { dir: PathBuf },
    /// Something is wrong with this copy (no payload, damaged, not Windows): why.
    Broken(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Language,
    /// evolite is installed already: its version against this one, before anything else.
    Existing,
    What,
    Space,
    Confirm,
    Warning,
    /// evolite runs (or hangs after a crash): close it, or let the installer end it.
    Running,
    Working,
    Done,
    Failed(String),
}

pub struct Setup {
    mode: Mode,
    step: Step,
    p: Palette,
    icons: Icons,
    fonts: Option<Lang>,
    known: Result<Known, String>,
    // Installation.
    agree: bool,
    desktop: bool,
    folder: String,
    disks: Vec<Disk>,
    disks_read: Option<Instant>,
    launch: bool,
    installed_dir: Option<PathBuf>,
    /// evolite as already installed, found when the installer starts.
    existing: Option<plan::Installed>,
    /// Its processes when it still runs ([`Step::Running`]); empty when only its lock says so.
    running: Vec<u32>,
    /// «Close evolite» did not end every process.
    not_ended: bool,
    // Removal.
    delete_data: bool,
    sure: bool,
    wallets: Vec<(PathBuf, Vec<FoundWallet>)>,
    data_bytes: u64,
    cache_bytes: u64,
    progress: Option<Shared>,
    /// `--screen`: a screen with the Figma example on it, for screenshots; nothing is read
    /// from the system and nothing is written.
    example: bool,
    /// `--screenshot <file.png>`: save the window after a few frames and quit.
    screenshot: Option<String>,
    frames: u32,
}

/// Text in the interface language, Arabic joined and in visual order.
fn txt(s: &str, size: f32, w: Weight, color: Color32) -> RichText {
    RichText::new(i18n::t(s)).font(sans(size, w)).color(color)
}

/// A template with its blanks filled: the template is the catalog's key, so it is translated
/// first and the values (paths, sizes) go in after.
fn fill(template: &str, values: &[&str]) -> String {
    let mut out = i18n::t_in(i18n::current(), template).into_owned();
    for v in values {
        out = out.replacen("{}", v, 1);
    }
    out
}

impl Setup {
    pub fn new(cc: &eframe::CreationContext<'_>, mode: Mode, lang: Lang) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        cc.egui_ctx.add_plugin(crate::rtl::Mirror::default());
        i18n::set(lang);
        // Before the first frame: a family drawn before its fonts are in panics.
        theme::install_fonts(&cc.egui_ctx, lang, true);
        let dark = cc.egui_ctx.system_theme() != Some(egui::Theme::Light);
        let p = if dark { theme::DARK } else { theme::LIGHT };
        theme::apply_visuals(&cc.egui_ctx, &p);
        let known = system::known();
        let folder = known
            .as_ref()
            .map(|k| k.default_program().display().to_string())
            .unwrap_or_default();
        let step = match &mode {
            Mode::Install { .. } => Step::Language,
            Mode::Uninstall { .. } => Step::Confirm,
            Mode::Broken(why) => Step::Failed(why.clone()),
        };
        let mut setup = Self {
            mode,
            step,
            p,
            icons: Icons::load(),
            fonts: Some(lang),
            known,
            agree: false,
            desktop: false,
            folder,
            disks: Vec::new(),
            disks_read: None,
            launch: true,
            installed_dir: None,
            existing: None,
            running: Vec::new(),
            not_ended: false,
            delete_data: false,
            sure: false,
            wallets: Vec::new(),
            data_bytes: 0,
            cache_bytes: 0,
            progress: None,
            example: false,
            screenshot: None,
            frames: 0,
        };
        // An installation already here: shown after the language, and its folder is where the
        // new one goes — a reinstall replaces it, whatever the versions.
        if let (Mode::Install { .. }, Ok(known)) = (&setup.mode, &setup.known) {
            setup.existing = system::installed(known);
            if let Some(found) = &setup.existing {
                crate::log(&format!(
                    "installed: {} version {:?} complete {}",
                    found.dir.display(),
                    found.version,
                    found.complete
                ));
                setup.folder = found.dir.display().to_string();
            }
        }
        if let (Mode::Uninstall { .. }, Ok(known)) = (&setup.mode, &setup.known) {
            setup.wallets = known
                .data_folders()
                .into_iter()
                .map(|d| {
                    let w = plan::wallets_in(&d);
                    (d, w)
                })
                .collect();
            setup.data_bytes = known.data_folders().iter().map(|d| plan::size_of(d)).sum();
            setup.cache_bytes = plan::size_of(&known.cache());
        }
        setup
    }

    /// `--screenshot` without `--screen`: this computer's first screen, saved, then quit —
    /// to check that the installer starts (after a crash, with another renderer).
    pub fn with_screenshot(mut self, screenshot: Option<String>) -> Self {
        self.screenshot = screenshot;
        self
    }

    /// `--screen NAME`: the screen with the Figma example (Anna's folders, two disks, three
    /// wallets) instead of this computer's. `--screenshot` saves it.
    pub fn example(mut self, screen: &str, screenshot: Option<String>, dark: Option<bool>) -> Self {
        #[cfg(windows)]
        let (known, system, second) = (
            Known {
                local: PathBuf::from(r"C:\Users\Anna\AppData\Local"),
                roaming: PathBuf::from(r"C:\Users\Anna\AppData\Roaming"),
                programs: PathBuf::from(r"C:\Users\Anna\AppData\Local\Programs"),
            },
            (r"C:\", "Windows"),
            (r"D:\", "Data"),
        );
        #[cfg(target_os = "macos")]
        let (known, system, second) = (
            Known::of_home(Path::new("/Users/anna"), None, None),
            ("/", "Macintosh HD"),
            ("/Volumes/Data", "Data"),
        );
        #[cfg(all(unix, not(target_os = "macos")))]
        let (known, system, second) = (
            Known::of_home(Path::new("/home/anna"), None, None),
            ("/", ""),
            ("/media/anna/Data", "Data"),
        );
        let disk = |(root, label): (&str, &str), free_gb: f64, total_gb: u64| Disk {
            root: PathBuf::from(root),
            label: label.into(),
            free: (free_gb * (1u64 << 30) as f64) as u64,
            total: total_gb << 30,
        };
        let low = screen == "space-low";
        self.example = true;
        self.screenshot = screenshot;
        self.folder = known.default_program().display().to_string();
        self.disks = vec![
            if low {
                disk(system, 0.3, 237)
            } else {
                disk(system, 118.0, 476)
            },
            disk(second, 412.0, 931),
        ];
        self.disks_read = Some(Instant::now());
        self.mode = if screen.starts_with("remove")
            || matches!(screen, "confirm" | "warning" | "running-remove")
        {
            Mode::Uninstall {
                dir: known.default_program(),
            }
        } else {
            Mode::Install {
                source: PathBuf::new(),
                payload: super::payload::Payload::example(),
            }
        };
        self.wallets = vec![
            (
                known.testnet_data(),
                ["Main", "Savings", "Tests with Bob"]
                    .iter()
                    .map(|n| FoundWallet {
                        name: (*n).into(),
                        network: "testnet",
                    })
                    .collect(),
            ),
            (known.mainnet_data(), Vec::new()),
        ];
        self.data_bytes = 412 << 20;
        self.cache_bytes = 48 << 20;
        self.installed_dir = Some(known.default_program());
        self.known = Ok(known);
        self.agree = true;
        self.delete_data = matches!(screen, "warning" | "removing" | "removed");
        // The confirmation under the warning starts off, as for real; only the later screens
        // show it given.
        self.sure = matches!(screen, "removing" | "removed");
        let working = |step, copied| {
            let shared = Shared::default();
            if let Ok(mut p) = shared.lock() {
                p.step = step;
                p.copied = copied;
                p.total = 73 << 20;
            }
            shared
        };
        self.step = match screen {
            "what" => Step::What,
            "space" | "space-low" => Step::Space,
            "installing" => {
                self.progress = Some(working(1, 45 << 20));
                Step::Working
            }
            "removing" => {
                self.progress = Some(working(2, 0));
                Step::Working
            }
            "existing" | "existing-newer" | "existing-older" | "existing-damaged" => {
                let version = match screen {
                    "existing-newer" => Some("0.9.0".to_owned()),
                    "existing-older" => Some("0.0.9".to_owned()),
                    "existing-damaged" => None,
                    _ => Some(env!("CARGO_PKG_VERSION").to_owned()),
                };
                self.existing = Some(plan::Installed {
                    dir: self.installed_dir.clone().unwrap_or_default(),
                    version,
                    complete: screen != "existing-damaged",
                });
                Step::Existing
            }
            "running" | "running-remove" => {
                self.running = vec![4242];
                Step::Running
            }
            "installed" | "removed" => Step::Done,
            "confirm" => Step::Confirm,
            "warning" => Step::Warning,
            _ => Step::Language,
        };
        if let Some(dark) = dark {
            self.p = if dark { theme::DARK } else { theme::LIGHT };
        }
        self
    }

    fn size(&self, n: u64) -> String {
        let (_, decimal) = i18n::separators();
        let s = plan::bytes(n, decimal);
        i18n::t_in(i18n::current(), &s).into_owned()
    }

    fn program_bytes(&self) -> u64 {
        match &self.mode {
            Mode::Install { payload, .. } => payload.installed_size() + payload.stub_len,
            _ => 0,
        }
    }

    fn data_exists(&self) -> bool {
        self.known
            .as_ref()
            .is_ok_and(|k| k.data_folders().iter().any(|d| d.exists()))
    }

    fn refresh_disks(&mut self) {
        if !self.example
            && self
                .disks_read
                .is_none_or(|at| at.elapsed() > Duration::from_secs(3))
        {
            self.disks = system::disks();
            self.disks_read = Some(Instant::now());
        }
    }
}

// ---------------------------------------------------------------- small widgets

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

fn button(ui: &mut Ui, p: &Palette, label: &str, kind: Kind, enabled: bool) -> bool {
    let font = sans(14.0, Weight::SemiBold);
    let shown = i18n::t(label).into_owned();
    let w = ui
        .painter()
        .layout_no_wrap(shown.clone(), font.clone(), Color32::WHITE)
        .size()
        .x
        + 36.0;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 41.0), sense);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &shown));
    let hovered = enabled && resp.hovered();
    let (fill, stroke, color) = match kind {
        Kind::Primary => (
            if hovered {
                p.ink.gamma_multiply(0.9)
            } else {
                p.ink
            },
            Color32::TRANSPARENT,
            p.on_ink,
        ),
        Kind::Secondary => (if hovered { p.raised } else { p.surface }, p.line, p.text),
        Kind::Ghost => (
            if hovered {
                p.raised
            } else {
                Color32::TRANSPARENT
            },
            Color32::TRANSPARENT,
            p.text,
        ),
        Kind::Danger => (
            if hovered {
                p.danger.gamma_multiply(0.9)
            } else {
                Color32::TRANSPARENT
            },
            p.danger,
            if hovered { Color32::WHITE } else { p.danger },
        ),
    };
    let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        dim(fill),
        Stroke::new(1.0, dim(stroke)),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        shown,
        font,
        dim(color),
    );
    enabled && resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

fn checkbox(
    ui: &mut Ui,
    p: &Palette,
    icons: &mut Icons,
    on: &mut bool,
    label: &str,
    sub: Option<&str>,
) {
    let row = ui
        .horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            let (fill, stroke) = if *on {
                (p.brand, p.brand)
            } else {
                (p.surface, p.line)
            };
            ui.painter().rect(
                rect,
                CornerRadius::same(5),
                fill,
                Stroke::new(1.5, stroke),
                StrokeKind::Inside,
            );
            if *on {
                icons
                    .icon("check", p.on_ink, p, 14.0)
                    .paint_at(ui, Rect::from_center_size(rect.center(), vec2(14.0, 14.0)));
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(txt(label, 14.0, Weight::Medium, p.text)).wrap());
                if let Some(sub) = sub {
                    ui.add(
                        egui::Label::new(
                            txt(sub, 12.0, Weight::Regular, p.muted).line_height(Some(17.0)),
                        )
                        .wrap(),
                    );
                }
            });
        })
        .response;
    let resp = crate::a11y::row_click(ui, &row, "checkbox");
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, i18n::t(label))
    });
    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        *on = !*on;
    }
}

fn card(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(12))
}

fn divider(ui: &mut Ui, p: &Palette) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0, p.line);
}

/// An icon, a title and a muted line under it (what is installed or removed).
fn item(ui: &mut Ui, p: &Palette, icons: &mut Icons, icon: &str, title: &str, sub: &str) {
    Frame::new()
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.add(icons.icon(icon, p.text, p, 20.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(txt(title, 14.0, Weight::Medium, p.text));
                    ui.add(egui::Label::new(txt(sub, 12.0, Weight::Regular, p.muted)).truncate());
                });
            });
        });
}

fn notice(ui: &mut Ui, p: &Palette, icons: &mut Icons, icon: &str, text: &str, color: Color32) {
    Frame::new()
        .fill(p.raised)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(icons.icon(icon, color, p, 18.0));
                ui.add(
                    egui::Label::new(
                        txt(text, 13.0, Weight::Regular, p.text).line_height(Some(18.0)),
                    )
                    .wrap(),
                );
            });
        });
}

fn heading(ui: &mut Ui, p: &Palette, title: &str, lead: Option<&str>) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(txt(title, 20.0, Weight::SemiBold, p.text));
        if let Some(lead) = lead {
            ui.add(
                egui::Label::new(txt(lead, 13.0, Weight::Regular, p.muted).line_height(Some(19.0)))
                    .wrap(),
            );
        }
    });
}

fn section_label(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(
        RichText::new(i18n::t(text).to_uppercase())
            .font(sans(11.0, Weight::SemiBold))
            .color(p.muted),
    );
}

fn progress_bar(ui: &mut Ui, p: &Palette, fraction: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
    ui.painter().rect_filled(r, 4, p.raised);
    let mut done = r;
    done.set_width(r.width() * fraction.clamp(0.0, 1.0));
    ui.painter().rect_filled(done, 4, p.brand);
}

/// The step list of the work: done, now, still to come.
fn steps(ui: &mut Ui, p: &Palette, icons: &mut Icons, names: &[&str], done: usize, finished: bool) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        for (i, name) in names.iter().enumerate() {
            let (icon, color, w) = if i < done || finished {
                ("confirmed", p.accent, Weight::Medium)
            } else if i == done {
                ("sync", p.text, Weight::Medium)
            } else {
                ("pending", p.muted, Weight::Regular)
            };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(icons.icon(icon, color, p, 18.0));
                ui.label(txt(
                    name,
                    14.0,
                    w,
                    if i <= done || finished {
                        p.text
                    } else {
                        p.muted
                    },
                ));
            });
        }
    });
}

// ---------------------------------------------------------------- the window

impl eframe::App for Setup {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let lang = i18n::current();
        // The language list shows every language's own name, each in its own script.
        let names = self.step == Step::Language;
        theme::fonts_applied();
        if self.fonts != Some(lang) || names != theme::take_name_fonts() {
            theme::install_fonts(ui.ctx(), lang, names);
            self.fonts = Some(lang);
        }
        if names {
            theme::want_name_fonts();
        }
        // egui's own widgets (the folder field) in the palette shown.
        if ui.style().visuals.dark_mode != self.p.dark {
            theme::apply_visuals(ui.ctx(), &self.p);
        }
        self.take_screenshot(ui.ctx());
        if let Some(progress) = &self.progress {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
            let result = progress.lock().ok().and_then(|p| p.result.clone());
            if let Some(result) = result {
                crate::log(&format!("finished: {result:?}"));
                self.step = match result {
                    Ok(()) => Step::Done,
                    Err(why) => Step::Failed(why),
                };
                self.progress = None;
            }
        }
        let p = self.p;
        Panel::top("title")
            .exact_size(52.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .stroke(Stroke::new(1.0, p.line))
                    .inner_margin(Margin::symmetric(24, 0)),
            )
            .show(ui, |ui| self.title_bar(ui));
        Panel::bottom("footer")
            .exact_size(72.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .stroke(Stroke::new(1.0, p.line))
                    .inner_margin(Margin::symmetric(24, 0)),
            )
            .show(ui, |ui| self.footer(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin {
                left: 40,
                right: 40,
                top: 24,
                bottom: 16,
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 14.0;
                        self.content(ui);
                    });
            });
    }
}

impl Setup {
    /// `--screenshot`: the window after the fonts and the logo have come in, then quit.
    fn take_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else {
            return;
        };
        self.frames += 1;
        ctx.request_repaint();
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            let [w, h] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            let saved =
                image::save_buffer(&path, &rgba, w as u32, h as u32, image::ColorType::Rgba8);
            println!("screenshot {path}: {saved:?}");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.frames == 30 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
    }

    fn title_bar(&mut self, ui: &mut Ui) {
        let p = self.p;
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(self.icons.logo(Logo::Horizontal, &p, 26.0));
            let step = match self.step {
                Step::Language | Step::Existing => Some(1),
                Step::What => Some(2),
                Step::Space => Some(3),
                Step::Working if matches!(self.mode, Mode::Install { .. }) => Some(4),
                _ => None,
            };
            if let Some(n) = step {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(txt(
                        &fill(tr!("Step {} of {}"), &[&n.to_string(), "4"]),
                        12.0,
                        Weight::Medium,
                        p.muted,
                    ));
                });
            }
        });
    }

    fn content(&mut self, ui: &mut Ui) {
        match self.step.clone() {
            Step::Language => self.language(ui),
            Step::Existing => self.existing(ui),
            Step::Running => self.running(ui),
            Step::What => self.what(ui),
            Step::Space => self.space(ui),
            Step::Confirm => self.confirm(ui),
            Step::Warning => self.warning(ui),
            Step::Working => self.working(ui),
            Step::Done => self.done(ui),
            Step::Failed(why) => self.failed(ui, &why),
        }
    }

    fn footer(&mut self, ui: &mut Ui) {
        let p = self.p;
        let installing = matches!(self.mode, Mode::Install { .. });
        let close = |ui: &mut Ui| ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            match self.step {
                Step::Language
                | Step::Existing
                | Step::Running
                | Step::What
                | Step::Space
                | Step::Confirm => {
                    if button(ui, &p, tr!("Cancel"), Kind::Ghost, true) {
                        close(ui);
                    }
                }
                Step::Warning => {
                    // Everything but the data goes.
                    let keep = button(ui, &p, tr!("Keep the data"), Kind::Secondary, true);
                    if keep {
                        self.delete_data = false;
                        self.start_removal();
                    }
                }
                _ => {}
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                match self.step.clone() {
                    Step::Language => {
                        if button(ui, &p, tr!("Next"), Kind::Primary, true) {
                            self.step = if self.existing.is_some() {
                                Step::Existing
                            } else {
                                Step::What
                            };
                        }
                    }
                    Step::Existing => {
                        if button(ui, &p, self.reinstall_label(), Kind::Primary, true) {
                            self.step = Step::What;
                        }
                        if button(ui, &p, tr!("Back"), Kind::Secondary, true) {
                            self.step = Step::Language;
                        }
                    }
                    Step::Running => {
                        if button(ui, &p, tr!("Try again"), Kind::Primary, true) {
                            self.retry();
                        }
                        if !self.running.is_empty()
                            && button(ui, &p, tr!("Close evolite"), Kind::Danger, true)
                        {
                            self.end_evolite();
                        }
                    }
                    Step::What => {
                        if button(ui, &p, tr!("Next"), Kind::Primary, self.agree) {
                            self.step = Step::Space;
                        }
                        if button(ui, &p, tr!("Back"), Kind::Secondary, true) {
                            self.step = if self.existing.is_some() {
                                Step::Existing
                            } else {
                                Step::Language
                            };
                        }
                    }
                    Step::Space => {
                        let ok = self.fits() != Some(Room::TooLittle) && self.fits().is_some();
                        if button(ui, &p, tr!("Install"), Kind::Primary, ok) {
                            self.start_install();
                        }
                        if button(ui, &p, tr!("Back"), Kind::Secondary, true) {
                            self.step = Step::What;
                        }
                    }
                    Step::Confirm => {
                        if button(ui, &p, tr!("Remove"), Kind::Danger, true) {
                            if self.delete_data && self.data_exists() {
                                self.sure = false;
                                self.step = Step::Warning;
                            } else {
                                self.start_removal();
                            }
                        }
                    }
                    Step::Warning => {
                        if button(
                            ui,
                            &p,
                            tr!("Delete the data forever"),
                            Kind::Danger,
                            self.sure,
                        ) {
                            self.start_removal();
                        }
                        if button(ui, &p, tr!("Back"), Kind::Ghost, true) {
                            self.step = Step::Confirm;
                        }
                    }
                    Step::Working => {
                        button(ui, &p, tr!("Done"), Kind::Primary, false);
                    }
                    Step::Done => {
                        let label = if installing {
                            tr!("Done")
                        } else {
                            tr!("Close")
                        };
                        if button(ui, &p, label, Kind::Primary, true) {
                            if installing
                                && self.launch
                                && let Some(dir) = &self.installed_dir
                            {
                                // macOS: through the Finder's `open`, as the app, with its icon.
                                let mut start = if cfg!(target_os = "macos") {
                                    let mut c = std::process::Command::new("open");
                                    c.arg(dir.join("evolite.app"));
                                    c
                                } else {
                                    std::process::Command::new(dir.join(plan::APP))
                                };
                                let _ = start.current_dir(dir).spawn();
                            }
                            close(ui);
                        }
                    }
                    Step::Failed(_) => {
                        if button(ui, &p, tr!("Close"), Kind::Primary, true) {
                            close(ui);
                        }
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------ installer

    fn language(&mut self, ui: &mut Ui) {
        let p = self.p;
        heading(
            ui,
            &p,
            tr!("Installation language"),
            Some(tr!(
                "evolite will start in the same language; you can change it in Settings."
            )),
        );
        let width = (ui.available_width() - 24.0) / 3.0;
        for row in Lang::ALL.chunks(3) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for &lang in row {
                    let on = i18n::current() == lang;
                    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::RadioButton,
                            true,
                            on,
                            lang.native(),
                        )
                    });
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(10),
                        if on || resp.hovered() {
                            p.raised
                        } else {
                            p.surface
                        },
                        Stroke::new(1.0, if on { p.brand } else { p.line }),
                        StrokeKind::Inside,
                    );
                    ui.painter().text(
                        pos2(rect.left() + 14.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        i18n::visual(lang.native()),
                        theme::name_font(lang, 14.0, Weight::Medium),
                        p.text,
                    );
                    if on {
                        self.icons.icon("check", p.accent, &p, 16.0).paint_at(
                            ui,
                            Rect::from_center_size(
                                pos2(rect.right() - 20.0, rect.center().y),
                                vec2(16.0, 16.0),
                            ),
                        );
                    }
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        i18n::set(lang);
                    }
                }
            });
        }
    }

    fn what(&mut self, ui: &mut Ui) {
        let p = self.p;
        heading(
            ui,
            &p,
            tr!("What will be installed"),
            Some(tr!(
                "evolite is a light Dash wallet for testnet. Nothing is installed until you press Install on the next step."
            )),
        );
        let size = self.size(self.program_bytes());
        card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            item(
                ui,
                &p,
                &mut self.icons,
                "wallets",
                tr!("evolite and its picture helper"),
                &fill(
                    if cfg!(windows) {
                        tr!("evolite.exe and evolite-renderer.exe · {}")
                    } else {
                        tr!("evolite and evolite-renderer · {}")
                    },
                    &[&size],
                ),
            );
            divider(ui, &p);
            let (title, sub) = if cfg!(windows) {
                (tr!("Shortcut in the Start menu"), tr!("Start → evolite"))
            } else if cfg!(target_os = "macos") {
                (tr!("Shortcut in Launchpad"), tr!("Launchpad → evolite"))
            } else {
                (
                    tr!("Entry in the applications menu"),
                    tr!("Applications → evolite"),
                )
            };
            item(ui, &p, &mut self.icons, "home", title, sub);
            divider(ui, &p);
            item(
                ui,
                &p,
                &mut self.icons,
                "trash",
                tr!("Uninstaller"),
                tr!("Removes the program; wallet data only after a separate confirmation"),
            );
        });
        checkbox(
            ui,
            &p,
            &mut self.icons,
            &mut self.desktop,
            tr!("Shortcut on the desktop"),
            None,
        );
        notice(
            ui,
            &p,
            &mut self.icons,
            "lock",
            tr!(
                "No administrator rights needed: evolite is installed for you only, in your own folder. Other users of this computer and their folders are not touched."
            ),
            p.text,
        );
        checkbox(
            ui,
            &p,
            &mut self.icons,
            &mut self.agree,
            tr!("I have read what will be installed and agree to continue"),
            None,
        );
    }

    fn target(&self) -> PathBuf {
        plan::install_dir(Path::new(self.folder.trim()))
    }

    /// Whether evolite fits where it would go; `None` when the folder is not on a local disk.
    fn fits(&self) -> Option<Room> {
        let known = self.known.as_ref().ok()?;
        let target = self.target();
        if !target.is_absolute() {
            return None;
        }
        let program = plan::disk_of(&self.disks, &target)?;
        let data = plan::disk_of(&self.disks, &known.roaming)?;
        Some(plan::room(self.program_bytes(), program, data))
    }

    fn space(&mut self, ui: &mut Ui) {
        self.refresh_disks();
        let p = self.p;
        ui.spacing_mut().item_spacing.y = 10.0;
        heading(ui, &p, tr!("Program folder and disk space"), None);
        let Ok(known) = self.known.clone() else {
            return;
        };
        ui.label(txt(tr!("Program folder"), 12.0, Weight::Medium, p.muted));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let field = egui::TextEdit::singleline(&mut self.folder)
                .font(sans(14.0, Weight::Regular))
                .margin(Margin::symmetric(14, 11))
                .desired_width(ui.available_width() - 110.0);
            ui.add(field);
            if button(ui, &p, tr!("Browse…"), Kind::Secondary, true)
                && let Some(chosen) = system::pick_folder(&self.target())
            {
                self.folder = plan::install_dir(&chosen).display().to_string();
            }
        });
        let target = self.target();
        ui.add(
            egui::Label::new(txt(
                &fill(
                    tr!("evolite goes to {}. Wallet data is always kept in {}."),
                    &[
                        &target.display().to_string(),
                        &known.testnet_data().display().to_string(),
                    ],
                ),
                12.0,
                Weight::Regular,
                p.muted,
            ))
            .wrap(),
        );
        let program_disk = plan::disk_of(&self.disks, &target).cloned();
        let data_disk = plan::disk_of(&self.disks, &known.roaming).cloned();
        let letter = |d: &Option<Disk>| {
            d.as_ref()
                .map(|d| {
                    d.root
                        .display()
                        .to_string()
                        .trim_end_matches('\\')
                        .to_owned()
                })
                .unwrap_or_default()
        };
        let (pl, dl) = (letter(&program_disk), letter(&data_disk));
        let room = self.fits();
        // Mainnet is the person's choice later: its recommendation is shown, never required.
        let mainnet_fits = data_disk
            .as_ref()
            .is_some_and(|d| d.free >= plan::RECOMMENDED_FREE_MAINNET);
        let rows = [
            (
                tr!("Program"),
                fill(tr!("on disk {}"), &[&pl]),
                self.size(self.program_bytes()),
                p.text,
            ),
            (
                tr!("Data after the sync"),
                fill(tr!("on disk {} · testnet"), &[&dl]),
                format!("≈ {}", self.size(plan::TESTNET_SYNCED)),
                p.text,
            ),
            (
                tr!("Data for mainnet, if turned on"),
                fill(tr!("on disk {}"), &[&dl]),
                format!("≈ {}", self.size(plan::MAINNET_SYNCED)),
                p.text,
            ),
            (
                tr!("Recommended free space"),
                fill(tr!("on disk {} · testnet"), &[&dl]),
                self.size(plan::RECOMMENDED_FREE),
                if room == Some(Room::Plenty) {
                    p.accent
                } else {
                    p.danger
                },
            ),
            (
                tr!("Recommended free space with mainnet"),
                fill(tr!("on disk {}"), &[&dl]),
                self.size(plan::RECOMMENDED_FREE_MAINNET),
                if mainnet_fits { p.accent } else { p.muted },
            ),
        ];
        card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, (label, where_, value, color)) in rows.iter().enumerate() {
                if i > 0 {
                    divider(ui, &p);
                }
                Frame::new()
                    .inner_margin(Margin::symmetric(16, 7))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(txt(label, 13.0, Weight::Medium, p.text));
                            ui.label(
                                RichText::new(i18n::visual(where_))
                                    .font(sans(12.0, Weight::Regular))
                                    .color(p.muted),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(value.as_str())
                                        .font(mono(13.0, Weight::Medium))
                                        .color(*color),
                                );
                            });
                        });
                    });
            }
        });
        self.room_notice(ui, room, &program_disk, &data_disk, &pl, &dl);
        section_label(ui, &p, tr!("Disks"));
        let disks = self.disks.clone();
        card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, d) in disks.iter().enumerate() {
                if i > 0 {
                    divider(ui, &p);
                }
                let holds_data = data_disk.as_ref().is_some_and(|x| x.root == d.root);
                let short = holds_data && room == Some(Room::TooLittle);
                self.disk_row(ui, d, holds_data, short);
            }
        });
        if room.is_none() {
            notice(
                ui,
                &p,
                &mut self.icons,
                "warning",
                tr!(
                    "This folder is not on a local disk. Choose a folder on one of the disks above."
                ),
                p.danger,
            );
        }
    }

    /// Why evolite does not fit, or fits tightly — over the disk list, where it is seen.
    fn room_notice(
        &mut self,
        ui: &mut Ui,
        room: Option<Room>,
        program_disk: &Option<Disk>,
        data_disk: &Option<Disk>,
        pl: &str,
        dl: &str,
    ) {
        let p = self.p;
        match (room, program_disk, data_disk) {
            (Some(Room::TooLittle), Some(pd), Some(dd)) => {
                let program = self.program_bytes();
                let text = if pd.free < program + (64 << 20) {
                    fill(
                        tr!(
                            "Only {} is free on disk {}, and the program needs {}. Choose another folder or free up space."
                        ),
                        &[&self.size(pd.free), pl, &self.size(program)],
                    )
                } else {
                    fill(
                        tr!(
                            "Only {} is free on disk {}, and the wallet data needs about {} after the sync. Free up space there: wallet data is always kept on this disk."
                        ),
                        &[&self.size(dd.free), dl, &self.size(plan::TESTNET_SYNCED)],
                    )
                };
                notice(ui, &p, &mut self.icons, "warning", &text, p.danger);
            }
            (Some(Room::Tight), _, _) => {
                let text = fill(
                    tr!("evolite fits, but less than {} will stay free on disk {}."),
                    &[&self.size(plan::RECOMMENDED_FREE), dl],
                );
                notice(ui, &p, &mut self.icons, "info", &text, p.accent);
            }
            _ => {}
        }
    }

    fn disk_row(&mut self, ui: &mut Ui, d: &Disk, holds_data: bool, short: bool) {
        let p = self.p;
        Frame::new()
            .inner_margin(Margin::symmetric(16, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    // A drive: a rounded box with its light.
                    let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    let body = Rect::from_center_size(r.center(), vec2(16.0, 12.0));
                    ui.painter()
                        .rect_stroke(body, 3, Stroke::new(1.6, p.text), StrokeKind::Inside);
                    ui.painter().circle_filled(
                        pos2(body.right() - 4.0, body.center().y),
                        1.5,
                        p.text,
                    );
                    let free = fill(
                        tr!("{} free of {}"),
                        &[&self.size(d.free), &self.size(d.total)],
                    );
                    let color = if short { p.danger } else { p.muted };
                    let mut name = d
                        .root
                        .display()
                        .to_string()
                        .trim_end_matches('\\')
                        .to_owned();
                    if !d.label.is_empty() {
                        name = format!("{name}  {}", d.label);
                    }
                    if holds_data {
                        name = fill(tr!("{} · wallet data"), &[&name]);
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        ui.label(
                            RichText::new(i18n::visual(&name))
                                .font(sans(14.0, Weight::SemiBold))
                                .color(p.text),
                        );
                        let (bar, _) = ui.allocate_exact_size(
                            vec2(ui.available_width() - 200.0, 6.0),
                            Sense::hover(),
                        );
                        ui.painter().rect_filled(bar, 3, p.raised);
                        let mut used = bar;
                        used.set_width(bar.width() * d.used_fraction());
                        ui.painter()
                            .rect_filled(used, 3, if short { p.danger } else { p.muted });
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(i18n::visual(&free))
                                .font(sans(12.0, Weight::Medium))
                                .color(color),
                        );
                    });
                });
            });
    }

    fn start_install(&mut self) {
        let (Mode::Install { source, payload }, Ok(known)) = (&self.mode, &self.known) else {
            return;
        };
        if self.example {
            return;
        }
        let (source, payload, known) = (source.clone(), payload.clone(), known.clone());
        // The folder it goes to, and the one it was in if that is another.
        let mut dirs = vec![self.target()];
        dirs.extend(self.existing.as_ref().map(|e| e.dir.clone()));
        if self.still_running(&dirs) {
            return;
        }
        crate::log(&format!("install to {}", self.target().display()));
        let job = work::Install {
            source,
            payload,
            dir: self.target(),
            desktop: self.desktop,
            known,
            language: i18n::current().code(),
        };
        self.installed_dir = Some(job.dir.clone());
        let shared = Shared::default();
        self.progress = Some(shared.clone());
        self.step = Step::Working;
        std::thread::spawn(move || work::install(job, shared));
    }

    // ------------------------------------------------------------ uninstaller

    fn confirm(&mut self, ui: &mut Ui) {
        let p = self.p;
        let Mode::Uninstall { dir } = &self.mode else {
            return;
        };
        let dir = dir.display().to_string();
        let Ok(known) = self.known.clone() else {
            return;
        };
        heading(
            ui,
            &p,
            tr!("Remove evolite"),
            Some(tr!("This will be removed from this computer:")),
        );
        let cache = format!(
            "{} · {}",
            known.cache().display(),
            self.size(self.cache_bytes)
        );
        card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            item(ui, &p, &mut self.icons, "wallets", tr!("Program"), &dir);
            divider(ui, &p);
            let shortcuts = if cfg!(windows) {
                tr!("Start menu and desktop")
            } else if cfg!(target_os = "macos") {
                tr!("Launchpad and desktop")
            } else {
                tr!("Applications menu and desktop")
            };
            item(ui, &p, &mut self.icons, "home", tr!("Shortcuts"), shortcuts);
            divider(ui, &p);
            #[cfg(windows)]
            {
                item(
                    ui,
                    &p,
                    &mut self.icons,
                    "document",
                    tr!("Entry in Installed apps"),
                    &format!("HKCU\\{}", system::UNINSTALL_KEY),
                );
                divider(ui, &p);
            }
            item(
                ui,
                &p,
                &mut self.icons,
                "nft",
                tr!("Picture cache (NFTs and avatars)"),
                &cache,
            );
        });
        section_label(ui, &p, tr!("Wallet data"));
        let sub = fill(
            tr!(
                "Without this, wallets, their history and settings stay in {} and open again if you install evolite later."
            ),
            &[&known.testnet_data().display().to_string()],
        );
        checkbox(
            ui,
            &p,
            &mut self.icons,
            &mut self.delete_data,
            tr!("Also delete wallet data"),
            Some(&sub),
        );
    }

    fn warning(&mut self, ui: &mut Ui) {
        let p = self.p;
        Frame::new()
            .fill(p.danger)
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(18, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    ui.add(self.icons.icon("warning", Color32::WHITE, &p, 28.0));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.add(egui::Label::new(txt(tr!("Without the recovery phrase the money will be lost"), 18.0, Weight::Bold, Color32::WHITE)).wrap());
                        ui.add(egui::Label::new(txt(tr!("The wallets' encrypted phrases are deleted with the data. A wallet can then be restored only from its written-down phrase of 12 or 24 words."), 13.0, Weight::Regular, Color32::WHITE).line_height(Some(18.0))).wrap());
                    });
                });
            });
        section_label(ui, &p, tr!("Wallets that will be deleted"));
        let wallets = self.wallets.clone();
        card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut first = true;
            for (folder, found) in &wallets {
                let folder_s = folder.display().to_string();
                if found.is_empty() {
                    if !first {
                        divider(ui, &p);
                    }
                    first = false;
                    let none = fill(tr!("No wallets in {}"), &[&folder_s]);
                    Frame::new()
                        .inner_margin(Margin::symmetric(16, 9))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(i18n::visual(&none))
                                    .font(sans(13.0, Weight::Regular))
                                    .color(p.muted),
                            );
                        });
                }
                for (n, w) in found.iter().enumerate() {
                    if !first {
                        divider(ui, &p);
                    }
                    first = false;
                    let name = if w.name.is_empty() {
                        fill(tr!("Wallet {}"), &[&(n + 1).to_string()])
                    } else {
                        w.name.clone()
                    };
                    Frame::new()
                        .inner_margin(Margin::symmetric(16, 9))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                                let color = theme::WALLET_COLORS[n % theme::WALLET_COLORS.len()];
                                ui.painter().circle_filled(r.center(), 12.0, color);
                                let letter: String = name
                                    .chars()
                                    .next()
                                    .map(|c| c.to_uppercase().collect())
                                    .unwrap_or_default();
                                ui.painter().text(
                                    r.center(),
                                    Align2::CENTER_CENTER,
                                    i18n::visual(&letter),
                                    sans(11.0, Weight::SemiBold),
                                    theme::WALLET_LETTER,
                                );
                                ui.label(
                                    RichText::new(i18n::visual(&name))
                                        .font(sans(14.0, Weight::SemiBold))
                                        .color(p.text),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!("{} · {folder_s}", w.network))
                                                .font(sans(12.0, Weight::Regular))
                                                .color(p.muted),
                                        )
                                        .truncate(),
                                    );
                                });
                            });
                        });
                }
            }
        });
        let total = fill(
            tr!(
                "In total {} will be deleted: wallets, history, settings, journal, network headers and filters."
            ),
            &[&self.size(self.data_bytes)],
        );
        ui.add(
            egui::Label::new(
                RichText::new(i18n::visual(&total))
                    .font(sans(12.0, Weight::Regular))
                    .color(p.muted),
            )
            .wrap(),
        );
        checkbox(
            ui,
            &p,
            &mut self.icons,
            &mut self.sure,
            tr!(
                "I have written down the phrases of all these wallets and understand that without them the money cannot be recovered"
            ),
            None,
        );
    }

    fn start_removal(&mut self) {
        let (Mode::Uninstall { dir }, Ok(known)) = (&self.mode, &self.known) else {
            return;
        };
        if self.example {
            return;
        }
        let (dir, known) = (dir.clone(), known.clone());
        if self.still_running(std::slice::from_ref(&dir)) {
            return;
        }
        crate::log(&format!(
            "remove {} (data: {})",
            dir.display(),
            self.delete_data && self.sure
        ));
        let job = work::Remove {
            dir,
            data: self.delete_data && self.sure,
            known,
        };
        let shared = Shared::default();
        self.progress = Some(shared.clone());
        self.step = Step::Working;
        std::thread::spawn(move || work::remove(job, shared));
    }

    // ------------------------------------------------------------ both

    fn working(&mut self, ui: &mut Ui) {
        let p = self.p;
        let (step, copied, total) = self
            .progress
            .as_ref()
            .and_then(|s| s.lock().ok().map(|s| (s.step, s.copied, s.total)))
            .unwrap_or_default();
        if let Mode::Install { .. } = self.mode {
            let dir = self
                .installed_dir
                .clone()
                .unwrap_or_default()
                .display()
                .to_string();
            heading(
                ui,
                &p,
                tr!("Installing evolite"),
                Some(&fill(tr!("Copying files to {}"), &[&dir])),
            );
            progress_bar(
                ui,
                &p,
                if total == 0 {
                    0.0
                } else {
                    copied as f32 / total as f32
                },
            );
            ui.label(
                RichText::new(fill(
                    tr!("{} of {}"),
                    &[&self.size(copied), &self.size(total)],
                ))
                .font(mono(12.0, Weight::Medium))
                .color(p.muted),
            );
            steps(
                ui,
                &p,
                &mut self.icons,
                &[
                    tr!("Checked the disk space"),
                    tr!("Copying the program"),
                    tr!("Start menu shortcut"),
                    tr!("Entry in Installed apps"),
                ],
                step,
                false,
            );
        } else {
            heading(ui, &p, tr!("Removing evolite"), None);
            progress_bar(ui, &p, step as f32 / work::REMOVE_STEPS as f32);
            let data = if self.delete_data && self.sure {
                tr!("Deleting the wallet data")
            } else {
                tr!("Keeping the wallet data")
            };
            steps(
                ui,
                &p,
                &mut self.icons,
                &[
                    tr!("Removed the program and shortcuts"),
                    data,
                    tr!("Picture cache and the registry entry"),
                ],
                step,
                false,
            );
        }
    }

    fn done(&mut self, ui: &mut Ui) {
        let p = self.p;
        let installing = matches!(self.mode, Mode::Install { .. });
        ui.add_space(70.0);
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 14.0;
            let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 32.0, p.raised);
            self.icons.icon("check", p.accent, &p, 32.0).paint_at(ui, Rect::from_center_size(r.center(), vec2(32.0, 32.0)));
            if installing {
                ui.label(txt(tr!("evolite is installed"), 22.0, Weight::SemiBold, p.text));
                ui.add(egui::Label::new(txt(tr!("The shortcut is in the Start menu. You can remove evolite in Settings → Apps."), 13.0, Weight::Regular, p.muted)).wrap());
            } else {
                ui.label(txt(tr!("evolite is removed"), 22.0, Weight::SemiBold, p.text));
                let text = if self.delete_data && self.sure {
                    i18n::t_in(i18n::current(), tr!("Removed: the program, shortcuts, the registry entry, the picture cache and the wallet data. evolite keeps nothing more on this computer.")).into_owned()
                } else {
                    let data = self.known.as_ref().map(|k| k.testnet_data().display().to_string()).unwrap_or_default();
                    fill(tr!("Removed: the program, shortcuts, the registry entry and the picture cache. Wallet data is kept in {}."), &[&data])
                };
                ui.add(egui::Label::new(RichText::new(i18n::visual(&text)).font(sans(13.0, Weight::Regular)).color(p.muted)).wrap());
            }
        });
        if installing {
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                ui.allocate_ui(vec2(220.0, 24.0), |ui| {
                    checkbox(
                        ui,
                        &p,
                        &mut self.icons,
                        &mut self.launch,
                        tr!("Launch evolite"),
                        None,
                    );
                });
            });
        }
    }

    /// Whether evolite still runs from one of `dirs` (or holds its data folder): then the
    /// «still running» screen instead of the work, which would only fail half way.
    fn still_running(&mut self, dirs: &[PathBuf]) -> bool {
        let Ok(known) = &self.known else {
            return false;
        };
        let mut procs: Vec<u32> = dirs.iter().flat_map(|d| system::processes_in(d)).collect();
        procs.sort_unstable();
        procs.dedup();
        let busy = system::running(known)
            || !procs.is_empty()
            || dirs.iter().any(|d| system::in_use(&d.join(plan::APP)));
        if busy {
            crate::log(&format!("evolite is running: processes {procs:?}"));
            self.running = procs;
            self.step = Step::Running;
        }
        busy
    }

    /// «Try again» on the «still running» screen: the same work again.
    fn retry(&mut self) {
        self.not_ended = false;
        match self.mode {
            Mode::Install { .. } => self.start_install(),
            Mode::Uninstall { .. } => self.start_removal(),
            Mode::Broken(_) => {}
        }
    }

    /// «Close evolite»: ends evolite's processes from its own folder (a hung one after a crash
    /// does not close by itself), then tries the work again.
    fn end_evolite(&mut self) {
        if self.example {
            return;
        }
        for pid in std::mem::take(&mut self.running) {
            let ended = system::end_process(pid);
            crate::log(&format!("end process {pid}: {ended}"));
        }
        self.retry();
        // Still there after the attempt: say so instead of offering the button again.
        if self.step == Step::Running && !self.running.is_empty() {
            self.not_ended = true;
        }
    }

    fn reinstall_label(&self) -> &'static str {
        let Some(found) = &self.existing else {
            return tr!("Reinstall");
        };
        if !found.complete {
            return tr!("Reinstall");
        }
        match plan::against(found.version.as_deref(), env!("CARGO_PKG_VERSION")) {
            plan::Against::Older => tr!("Update"),
            plan::Against::Newer => tr!("Install the older version"),
            plan::Against::Same | plan::Against::Unknown => tr!("Reinstall"),
        }
    }

    /// evolite is installed already: both versions, what installing now does, and that the
    /// wallets stay. Installing is always offered — a reinstall repairs a broken installation.
    fn existing(&mut self, ui: &mut Ui) {
        let p = self.p;
        let Some(found) = self.existing.clone() else {
            return;
        };
        let this = env!("CARGO_PKG_VERSION");
        let against = plan::against(found.version.as_deref(), this);
        let newer = found.complete && against == plan::Against::Newer;
        heading(
            ui,
            &p,
            if newer {
                tr!("A newer version of evolite is installed")
            } else {
                tr!("evolite is already installed")
            },
            Some(tr!(
                "Installing replaces the program files. Wallets, their history and settings stay."
            )),
        );
        let installed_version = match &found.version {
            Some(v) => fill(tr!("Version {}"), &[v]),
            None => i18n::t(tr!("Version unknown")).into_owned(),
        };
        let installed = format!("{installed_version} · {}", found.dir.display());
        let carried = fill(tr!("Version {}"), &[this]);
        card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            item(
                ui,
                &p,
                &mut self.icons,
                "wallets",
                tr!("Installed now"),
                &installed,
            );
            divider(ui, &p);
            item(
                ui,
                &p,
                &mut self.icons,
                "document",
                tr!("In this installer"),
                &carried,
            );
        });
        let installed_v = found.version.clone().unwrap_or_default();
        let (icon, text, color) = if !found.complete {
            (
                "warning",
                i18n::t(tr!(
                    "The installation is incomplete: some of its files are missing. Reinstalling repairs it."
                ))
                .into_owned(),
                p.danger,
            )
        } else {
            match against {
                plan::Against::Same => (
                    "sync",
                    i18n::t(tr!(
                        "The same version is installed. You can install it again, for example to repair it."
                    ))
                    .into_owned(),
                    p.text,
                ),
                plan::Against::Older => (
                    "sync",
                    fill(
                        tr!("An older version is installed: it will be updated to {}."),
                        &[this],
                    ),
                    p.accent,
                ),
                plan::Against::Newer => (
                    "warning",
                    fill(
                        tr!(
                            "Installed: {}. This installer has an older version, {}. Installing it replaces the newer version with the older one."
                        ),
                        &[&installed_v, this],
                    ),
                    p.danger,
                ),
                plan::Against::Unknown => (
                    "warning",
                    fill(
                        tr!("The installed version is not known. Installing writes version {} over it."),
                        &[this],
                    ),
                    p.muted,
                ),
            }
        };
        notice(ui, &p, &mut self.icons, icon, &text, color);
    }

    /// evolite runs — or, after a crash, stays without a window or does not respond.
    fn running(&mut self, ui: &mut Ui) {
        let p = self.p;
        heading(
            ui,
            &p,
            tr!("evolite is still running"),
            Some(tr!(
                "Close evolite and press Try again. After a crash it may stay open without a window or stop responding: then Close evolite ends it."
            )),
        );
        if self.not_ended {
            let text = i18n::t(tr!(
                "evolite could not be closed. Restart the computer and start the installer again."
            ))
            .into_owned();
            notice(ui, &p, &mut self.icons, "warning", &text, p.danger);
        } else if self.running.is_empty() {
            let text = i18n::t(tr!(
                "evolite is open from another folder or by another program. Close it yourself and press Try again."
            ))
            .into_owned();
            notice(ui, &p, &mut self.icons, "warning", &text, p.muted);
        }
        notice(
            ui,
            &p,
            &mut self.icons,
            "lock",
            tr!("Wallets are kept on disk as they change: closing evolite does not remove them."),
            p.text,
        );
    }

    fn failed(&mut self, ui: &mut Ui, why: &str) {
        let p = self.p;
        let title = match self.mode {
            Mode::Uninstall { .. } => tr!("Removal did not finish"),
            _ => tr!("Installation did not finish"),
        };
        heading(ui, &p, title, None);
        let why = i18n::t(why).into_owned();
        notice(ui, &p, &mut self.icons, "warning", &why, p.danger);
    }
}
