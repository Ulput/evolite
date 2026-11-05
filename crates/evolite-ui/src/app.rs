//! The app: the core's handle, its last snapshot, what each screen keeps between frames, and
//! the frame itself — which screen is open, the sidebar, and the theme.

#[cfg(feature = "dev-tools")]
mod dev;
// The whole app on the demo core, driven by simulated clicks and keys (egui_kittest). It
// opens screens by their `--screen` presets, which exist only with `dev-tools`.
#[cfg(all(test, demo_core, feature = "dev-tools"))]
mod ui_tests;

use crate::cli::{CoreChoice, Launch};
use crate::components::{Assets, txt};
use crate::context::Ctx;
use crate::screens::Screen;
use crate::screens::onboarding::{Onboarding, Outcome};
use crate::screens::settings::{Section, ThemeChoice};
use crate::{icons, screens, theme};
use egui::{Frame, Margin, Panel, Stroke};
use evolite_api::{Command, Event, Snapshot, VaultState};
use evolite_core::Core;

pub struct App {
    a: Assets,
    pub core: Core,
    /// The last snapshot from the core; `None` until the first one arrives.
    snapshot: Option<Snapshot>,
    screen: Screen,
    filter: usize,
    query: String,
    send: screens::send::SendForm,
    receive: screens::receive::ReceiveForm,
    identity: screens::identity::IdentityState,
    contacts: screens::contacts::ContactsState,
    pool: screens::shielded::PoolState,
    requests: screens::app_requests::AppRequests,
    collection: screens::collection::CollectionState,
    settings: screens::settings::SettingsState,
    create_identity: screens::create_identity::CreateIdentityForm,
    unlock: screens::unlock::UnlockForm,
    onboarding: Option<Onboarding>,
    /// The screenshot tools' state (`dev-tools`).
    #[cfg(feature = "dev-tools")]
    dev: dev::DevState,
    /// When the core was last told the person is here (for its auto-lock).
    touched: Option<std::time::Instant>,
    /// The system's "show animations", and when it was read.
    system_motion: Option<(std::time::Instant, bool)>,
    /// The vault as the last snapshot had it: a lock is seen once, when it happens.
    vault: Option<VaultState>,
    /// What the core was last told of "New address…" and "Fiat rate".
    core_told: Option<(bool, Option<&'static str>)>,
    /// For `on_exit`, which gets no context: the copied phrase is wiped there too.
    egui_ctx: Option<egui::Context>,
    /// The data folder; `None` in the demo. Its `settings.json` holds the app's settings, each
    /// wallet's folder that wallet's. What both hold now, together.
    data_dir: Option<std::path::PathBuf>,
    saved_prefs: crate::prefs::Prefs,
    /// The app's `settings.json` as read at the start: a file from before also holds the
    /// one wallet's settings, taken by that wallet ([`crate::prefs::load_wallet`]).
    read_prefs: crate::prefs::Prefs,
    /// The wallet whose settings are in [`Self::settings`] now; `None` before the first
    /// snapshot and while a wallet is being added.
    prefs_wallet: Option<String>,
    /// the wallet menu under the switcher is open.
    wallet_menu: bool,
    /// Which visit of the screen this is: its widgets, and so its scroll position, live under
    /// this number (BUG-48).
    visits: Visits<Screen>,
    /// "Add wallet" asked: the onboarding to open (none: the welcome screen) once the
    /// core has made room for the new wallet.
    adding: Option<Option<screens::onboarding::Flow>>,
    /// "Delete" on a wallet that was not open: its deletion opens over its unlock screen
    /// (`true`: "Forgot the password?", without the password).
    delete_other: Option<(String, bool)>,
    /// "Open "…"" over the screen (Figma 29), while open.
    switching: Option<screens::switch_wallet::SwitchForm>,
    /// While a wallet is being added: the name of the one "‹ Back to "…"" goes back to.
    adding_back: Option<String>,
    /// Wallets already given their default name ([`Self::name_unnamed`]): asked once each.
    named: std::collections::HashSet<String>,
    /// The identity worked as, kept between runs; asked for again after the unlock
    /// until `identity_restored`.
    chosen_identity: Option<String>,
    identity_restored: bool,
    /// the system notifications, and the history's proofs they were last compared with.
    notifier: crate::notify::Notifier,
    seen: Option<std::collections::HashMap<u64, evolite_api::Proof>>,
    /// What the fonts were installed for: the language (Chinese, Japanese and Korean need their
    /// own) and whether the language picker was open (it shows every language's name).
    fonts: (crate::i18n::Lang, bool),
    /// Whether the window is kept out of screen captures now (a phrase is on screen).
    capture_hidden: bool,
    /// Screens and long frames for the journal.
    watch: crate::diagnostics::FrameWatch,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        let art = evolite_core::ArtOptions {
            cache: Some(crate::cli::cache_for(&launch.core)),
            ..Default::default()
        };
        Self::with_art(cc, launch, art)
    }

    /// [`Self::new`] with NFT pictures drawn and kept as `art` says; the UI tests pass
    /// [`evolite_core::ArtOptions::off`].
    pub(crate) fn with_art(
        cc: &eframe::CreationContext<'_>,
        launch: Launch,
        art: evolite_core::ArtOptions,
    ) -> Self {
        #[cfg(feature = "dev-tools")]
        let mut launch = launch;
        egui_extras::install_image_loaders(&cc.egui_ctx);
        // Arabic: the whole frame mirrored (menu on the right), see `rtl`.
        cc.egui_ctx.add_plugin(crate::rtl::Mirror::default());
        // Before the mirror's output: the ring is mirrored with the frame.
        cc.egui_ctx.add_plugin(crate::focus::FocusRing::default());
        let p = if launch.dark {
            theme::DARK
        } else {
            theme::LIGHT
        };
        theme::apply_visuals(&cc.egui_ctx, &p);
        let repaint = cc.egui_ctx.clone();
        #[cfg(demo_core)]
        let demo = matches!(launch.core, CoreChoice::Mock) || !cfg!(feature = "real");
        #[cfg(not(demo_core))]
        let demo = false;
        // "Own node" is set: for the journal, whether only.
        let own_node = match &launch.core {
            CoreChoice::Real(dir) => dir.join("core-node.txt").exists(),
            #[cfg(demo_core)]
            CoreChoice::Mock => false,
        };
        // `--window`: Windows may open the window maximized whatever size was asked for.
        #[cfg(feature = "dev-tools")]
        if let Some([w, h]) = launch.dev.window {
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
        }
        let data_dir = match &launch.core {
            CoreChoice::Real(dir) if !demo => Some(dir.clone()),
            _ => None,
        };
        let core = match launch.core {
            // The demo on mainnet only for screenshots of how mainnet looks; it has no wallet.
            #[cfg(demo_core)]
            CoreChoice::Mock => {
                evolite_core::network::choose(launch.network);
                Core::mock(art, move || repaint.request_repaint())
            }
            #[cfg(feature = "real")]
            CoreChoice::Real(ref dir) => {
                Core::real_on(launch.network, dir.clone(), art, move || {
                    repaint.request_repaint()
                })
                .expect("one network per process")
            }
            // Built without the real core: the demo is all there is.
            #[cfg(not(feature = "real"))]
            CoreChoice::Real(_) => Core::mock(art, move || repaint.request_repaint()),
        };
        // The Figma example only with the example wallet.
        let send = if demo {
            screens::send::SendForm {
                previewed: "@alice".into(),
                ..Default::default()
            }
        } else {
            screens::send::SendForm::empty()
        };
        let mut app = Self {
            a: Assets {
                p,
                icons: icons::Icons::load(),
                avatars: Default::default(),
            },
            core,
            unlock: Default::default(),
            snapshot: None,
            screen: launch.screen,
            filter: 0,
            query: String::new(),
            send,
            receive: Default::default(),
            requests: Default::default(),
            identity: Default::default(),
            contacts: Default::default(),
            pool: Default::default(),
            collection: Default::default(),
            settings: screens::settings::SettingsState::new(launch.dark, Section::Network, false),
            create_identity: Default::default(),
            #[cfg(feature = "dev-tools")]
            dev: dev::DevState::new(&launch.dev, launch.screen),
            system_motion: None,
            #[cfg(feature = "dev-tools")]
            onboarding: launch.dev.onboarding.take(),
            #[cfg(not(feature = "dev-tools"))]
            onboarding: None,
            touched: None,
            vault: None,
            egui_ctx: None,
            data_dir,
            saved_prefs: Default::default(),
            read_prefs: Default::default(),
            prefs_wallet: None,
            wallet_menu: false,
            visits: Visits::default(),
            adding: None,
            delete_other: None,
            switching: None,
            adding_back: None,
            named: Default::default(),
            core_told: None,
            notifier: Default::default(),
            seen: None,
            chosen_identity: None,
            identity_restored: false,
            fonts: (crate::i18n::Lang::Ru, false),
            capture_hidden: false,
            watch: Default::default(),
        };
        #[cfg(feature = "dev-tools")]
        app.apply_presets(&cc.egui_ctx, &launch.dev, demo);
        if let Some(dir) = &app.data_dir {
            let saved = crate::prefs::load(&crate::prefs::path(dir));
            app.settings.apply(&saved, &mut app.collection.trusted);
            app.chosen_identity = saved.identity.clone();
            app.read_prefs = saved;
        }
        #[cfg(feature = "dev-tools")]
        app.apply_settings_presets(&launch.dev);
        app.saved_prefs = app.settings.prefs(&app.collection.trusted);
        app.saved_prefs.identity = app.chosen_identity.clone();
        // The language: `--lang`, else the one chosen, else the system's; the example wallet
        // stays Russian, as in Figma, unless asked.
        let lang = launch.lang.or(app.settings.lang).unwrap_or_else(|| {
            if demo {
                crate::i18n::Lang::Ru
            } else {
                crate::i18n::system()
            }
        });
        crate::i18n::set(lang);
        theme::install_fonts(&cc.egui_ctx, lang, false);
        app.fonts = (lang, false);
        app.tell_core_settings();
        evolite_core::diag::record(evolite_core::diag::Record::Start(
            crate::diagnostics::build(
                lang.code(),
                app.a.p.dark,
                cc.egui_ctx.pixels_per_point(),
                evolite_core::network::current(),
                own_node,
                demo,
            ),
        ));
        #[cfg(feature = "dev-tools")]
        app.unlock_for_shot();
        app
    }

    /// The script check (`--screen probe`, dev-tools) is open.
    fn probe_open(&self) -> bool {
        #[cfg(feature = "dev-tools")]
        return self.screen == Screen::Probe;
        #[cfg(not(feature = "dev-tools"))]
        false
    }

    /// `--open` (dev-tools) only looks: it writes no settings and names no wallet.
    fn looking_only(&self) -> bool {
        #[cfg(feature = "dev-tools")]
        return self.dev.real_shot;
        #[cfg(not(feature = "dev-tools"))]
        false
    }

    /// what is new in `snapshot` goes to the system's notifications while the window is
    /// not in front (in front, the screens show it). The demo's example history never does.
    fn notify_changes(&mut self, ctx: &egui::Context, snapshot: &Snapshot) {
        let Some(wallet) = snapshot
            .wallet
            .as_ref()
            .filter(|_| !crate::is_demo(snapshot))
        else {
            self.seen = None;
            return;
        };
        let wanted = crate::notify::Wanted {
            incoming: self.settings.is_on("notify-incoming"),
            locks: self.settings.is_on("notify-locks"),
        };
        let notes = crate::notify::changes(
            self.seen.as_ref(),
            &wallet.activity,
            chrono::Utc::now().timestamp(),
            wanted,
        );
        self.seen = Some(crate::notify::proofs(&wallet.activity));
        if !ctx.input(|i| i.focused) {
            for note in &notes {
                self.notifier.show(note);
            }
        }
    }

    /// The settings the core acts on (auto-lock, the receive address, the rate), sent whole:
    /// at the start and to a new core after a deletion.
    fn tell_core_settings(&mut self) {
        for command in self.settings.core_commands() {
            self.core.send(command);
        }
        self.core_told = Some(self.core_settings());
    }

    /// The part of [`Self::tell_core_settings`] a switch changes in Settings.
    fn core_settings(&self) -> (bool, Option<&'static str>) {
        (
            self.settings.is_on("new-address"),
            self.settings.fiat_currency(),
        )
    }

    /// Writes the settings when they changed since the last write (a click, not every frame).
    fn save_prefs(&mut self) {
        // `--open` looks and changes nothing, not even the settings its presets show.
        if self.looking_only() {
            return;
        }
        let Some(dir) = &self.data_dir else {
            return;
        };
        let mut now = self.settings.prefs(&self.collection.trusted);
        now.identity = self.chosen_identity.clone();
        if now == self.saved_prefs {
            return;
        }
        // the app's part to the data folder, the wallet's to its folder. With no wallet
        // open, only the app's — but an old file not split yet is written whole, so its wallet
        // part is not lost before its wallet takes it.
        let (app, wallet) = crate::prefs::split(&now);
        let app_file = crate::prefs::path(dir);
        let written = match &self.prefs_wallet {
            Some(id) => crate::prefs::save(&app_file, &app)
                .and_then(|()| crate::prefs::save(&crate::prefs::wallet_path(dir, id), &wallet)),
            None if self.read_prefs != crate::prefs::Prefs::default() => {
                crate::prefs::save(&app_file, &now)
            }
            None => crate::prefs::save(&app_file, &app),
        };
        if let Err(e) = written {
            eprintln!("settings not saved: {e}");
        }
        self.saved_prefs = now;
    }

    /// a wallet the core added without a name (new, or moved from an old folder) is
    /// named once, in the interface language of the moment. `--open` changes nothing.
    fn name_unnamed(&mut self, snapshot: &Snapshot) {
        if self.looking_only() {
            return;
        }
        for (place, w) in snapshot.wallets.iter().enumerate() {
            if w.name.is_empty() && self.named.insert(w.id.clone()) {
                self.core.send(Command::RenameWallet {
                    id: w.id.clone(),
                    name: screens::wallets::default_name(place),
                    color: w.color,
                });
            }
        }
    }

    /// the snapshot's open wallet is not the one whose settings are shown — the first
    /// snapshot, a switch, an added or a deleted wallet. Its own settings come in; the app's
    /// stay. A wallet without a file keeps what is on screen when nothing was open before (a
    /// new wallet keeps what its onboarding chose; the first wallet after an update keeps the
    /// old file's), else it starts from the defaults.
    fn follow_open_wallet(&mut self, snapshot: &Snapshot) {
        let open = snapshot
            .wallets
            .iter()
            .find(|w| w.open)
            .map(|w| w.id.clone());
        if open == self.prefs_wallet || crate::is_demo(snapshot) {
            return;
        }
        let before = self.prefs_wallet.take();
        let switched = before.is_some();
        let loaded = match (&self.data_dir, &open) {
            (Some(dir), Some(id)) => crate::prefs::load_wallet(dir, id, &self.read_prefs),
            _ => None,
        };
        // After the first time, the old file's wallet part belongs to no other wallet.
        self.read_prefs = Default::default();
        if switched || loaded.is_some() {
            self.settings.reset_wallet();
            self.collection.trusted.clear();
            self.chosen_identity = None;
            self.identity_restored = false;
            if let Some(saved) = &loaded {
                self.settings.apply(saved, &mut self.collection.trusted);
                self.chosen_identity = saved.identity.clone();
            }
        }
        if switched {
            // A new core over the other wallet knows none of the settings it acts on.
            self.tell_core_settings();
            self.wallet_menu = false;
            self.unlock = Default::default();
        }
        self.prefs_wallet = open;
        // Written at once: each part to its file (an old file is split now).
        self.saved_prefs = Default::default();
    }

    /// Any input means the person is here: the core's auto-lock starts again. Told at most
    /// every few seconds, not every frame.
    fn touch(&mut self, ctx: &egui::Context) {
        let active = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::PointerMoved(_)
                        | egui::Event::PointerButton { .. }
                        | egui::Event::MouseWheel { .. }
                        | egui::Event::Key { .. }
                        | egui::Event::Text(_)
                )
            })
        });
        let due = self
            .touched
            .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(5));
        if active && due {
            self.core.send(Command::Touch);
            self.touched = Some(std::time::Instant::now());
        }
    }

    /// Hands each answer from the core to whoever asked.
    fn take_events(&mut self, ctx: &egui::Context) {
        for event in self.core.events() {
            match &event {
                Event::Snapshot(snapshot) => {
                    self.follow_open_wallet(snapshot);
                    self.name_unnamed(snapshot);
                    self.notify_changes(ctx, snapshot);
                    self.snapshot = Some((**snapshot).clone());
                    // Shown with its default name until the rename lands — and always under
                    // `--open`, which writes nothing (a wallet made by the core alone, e.g. a
                    // live test's restore, has no name in the index).
                    if let Some(s) = self.snapshot.as_mut() {
                        screens::wallets::show_default_names(&mut s.wallets);
                    }
                    #[cfg(feature = "dev-tools")]
                    self.dev_snapshot(ctx);
                }
                Event::NameResolved { name, result } => {
                    if *name == self.send.asked {
                        self.send.resolution = Some(result.clone());
                    }
                    self.contacts.on_resolved(name, result);
                }
                Event::Done {
                    what: "contact",
                    result,
                } => {
                    self.contacts.on_done(result);
                    self.send.on_contact_done(result);
                }
                Event::Done {
                    what: "address-book",
                    result,
                } => self.send.on_book_done(result),
                Event::Done {
                    what: "profile",
                    result,
                } => self.identity.on_profile_saved(result),
                Event::Sent(result) => self.send.on_sent(result),
                Event::PaymentPreview { to, result } => self.send.on_preview(to, result),
                Event::Done {
                    what: "gift",
                    result,
                } => self.collection.result = Some(result.clone()),
                Event::Done {
                    what: "unlock",
                    result,
                } => self.unlock.on_answer(result),
                // Another wallet by its password: from "Open "…"" or the unlock screen.
                Event::Done {
                    what: "switch",
                    result,
                } => match &mut self.switching {
                    Some(form) => {
                        if form.on_done(result) {
                            self.switching = None;
                        }
                    }
                    None => self.unlock.on_answer(result),
                },
                // The resync is marked: evolite starts again with the same arguments and this
                // window closes. The new one waits for this one to let go of the folder.
                Event::Done {
                    what: "resync",
                    result,
                } => {
                    let marked = self
                        .settings
                        .resync
                        .as_mut()
                        .is_some_and(|form| form.on_done(result));
                    if marked {
                        start_again(ctx);
                    }
                }
                // The wallet is gone: both deletion forms close, the welcome screen follows.
                Event::Done {
                    what: "delete",
                    result,
                } => {
                    let mut gone = false;
                    for form in [&mut self.settings.delete, &mut self.unlock.forgot]
                        .into_iter()
                        .flatten()
                    {
                        gone |= form.on_done(result);
                    }
                    if gone {
                        self.settings.delete = None;
                        self.unlock.forgot = None;
                        self.screen = Screen::Welcome;
                        // What belonged to the deleted wallet goes with it: the day its
                        // phrase was checked, the identity chosen, the NFT issuers trusted. The
                        // person's own choices (theme, auto-lock, switches) stay.
                        self.settings.forget_wallet();
                        self.chosen_identity = None;
                        self.collection.trusted.clear();
                    }
                }
                Event::PhraseRevealed(result) => self.settings.on_revealed(result.clone()),
                Event::PhraseCheck(result) => self.settings.on_phrase_questions(result),
                Event::PhraseChecked { wrong } => self
                    .settings
                    .on_phrase_checked(*wrong, chrono::Utc::now().timestamp()),
                Event::Done {
                    what: "password",
                    result,
                } => self.settings.on_password_changed(result),
                Event::Done {
                    what: "wallet",
                    result,
                } => {
                    if self
                        .settings
                        .rename
                        .as_mut()
                        .is_some_and(|form| form.on_done(result))
                    {
                        self.settings.rename = None;
                    }
                }
                Event::Done {
                    what: "core-node",
                    result,
                } => self.settings.on_node_saved(result),
                Event::AppRequest(request) => {
                    if let Ok(request) = request
                        && self.settings.is_on("notify-apps")
                        && !ctx.input(|i| i.focused)
                    {
                        self.notifier
                            .show(&crate::notify::app_request(&request.app));
                    }
                    self.requests.on_request(request.clone());
                }
                Event::Done {
                    what: "app",
                    result,
                } => self.requests.on_answer(result.clone()),
                Event::Done {
                    what: "disconnect",
                    result,
                } => self.requests.on_disconnected(result.clone()),
                // "Hide" and "Unshield" share the pool's progress: one move at a time.
                Event::Progress {
                    what: "shield" | "unshield" | "pool-top-up",
                    fraction,
                } => self.pool.progress = Some(*fraction),
                Event::Done {
                    what: "shield" | "unshield" | "pool-top-up",
                    result,
                } => {
                    self.pool.progress = None;
                    self.pool.result = Some(result.clone());
                }
                Event::NameChecked { label, result } => {
                    if *label == self.create_identity.asked {
                        self.create_identity.check = Some(result.clone());
                    }
                    self.identity.on_name_checked(label, result);
                }
                Event::Done {
                    what: "name",
                    result,
                } => self.identity.on_name_done(result),
                Event::Done {
                    what: "identity",
                    result,
                } => self.create_identity.on_done(result),
                // Which of the four steps of "Create an identity" the core is on.
                Event::Progress {
                    what: "identity",
                    fraction,
                } => self.create_identity.on_progress(*fraction),
                Event::Done {
                    what: "top-up",
                    result,
                } => self.identity.on_top_up_done(result),
                Event::Art {
                    art,
                    side,
                    partial,
                    result,
                } => {
                    let cache = if art.starts_with("avatar:") {
                        &mut self.a.avatars
                    } else {
                        &mut self.collection.art
                    };
                    cache.on_answer(ctx, art, *side, *partial, result);
                }
                _ => {}
            }
            if let Some(o) = &mut self.onboarding {
                o.on_event(&event);
            }
        }
    }
}

/// Counts the visits of a place (a screen, a settings section): a new number each time the
/// place shown changes, the same while it stays. A scroll area under that number opens at the
/// top on every visit and keeps its place while the person stays (BUG-48).
pub(crate) struct Visits<T> {
    shown: Option<T>,
    n: u32,
}

impl<T> Default for Visits<T> {
    fn default() -> Self {
        Self { shown: None, n: 0 }
    }
}

impl<T: PartialEq + Copy> Visits<T> {
    /// The visit `now` is in: a new one when the last call showed another place.
    pub(crate) fn of(&mut self, now: T) -> u32 {
        if self.shown != Some(now) {
            self.shown = Some(now);
            self.n = self.n.wrapping_add(1);
        }
        self.n
    }
}

/// The open wallet's keys went since the last snapshot: locked, or deleted (a deletion goes from
/// open straight to no wallet, never through "locked").
fn keys_gone(before: Option<VaultState>, now: VaultState) -> bool {
    before == Some(VaultState::Unlocked) && now != VaultState::Unlocked
}

/// evolite starts again with the same arguments and this window closes; the new one waits
/// for this one to let go of the folder. After a resync, or for a new Core node.
fn start_again(ctx: &egui::Context) {
    let started = std::env::current_exe().and_then(|exe| {
        std::process::Command::new(exe)
            .args(std::env::args_os().skip(1))
            .spawn()
    });
    match started {
        Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        Err(e) => eprintln!("evolite did not start again: {e}"),
    }
}

impl eframe::App for App {
    /// The phrase does not outlive the app on the clipboard.
    fn on_exit(&mut self) {
        if let Some(ctx) = &self.egui_ctx {
            crate::clipboard::wipe_now(ctx);
        }
        self.save_prefs();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let frame_started = std::time::Instant::now();
        self.egui_ctx.get_or_insert_with(|| ui.ctx().clone());
        crate::clipboard::set_wipe_copies(self.settings.is_on("clipboard"));
        crate::clipboard::tick(ui.ctx());
        // A language chosen in Settings takes its fonts with it; the open language picker
        // needs the fonts of every language's name.
        theme::fonts_applied();
        let fonts = (crate::i18n::current(), theme::take_name_fonts());
        if fonts != self.fonts {
            theme::install_fonts(ui.ctx(), fonts.0, fonts.1);
            self.fonts = fonts;
            ui.ctx().request_repaint();
        }
        // What the last frame changed in Settings goes to disk now.
        self.save_prefs();
        self.take_events(ui.ctx());
        // Settings → Collection: whether NFT pictures move, and whether a stranger's picture waits
        // for "Show". The system's choice is read again every few seconds.
        let now = std::time::Instant::now();
        if self
            .system_motion
            .is_none_or(|(at, _)| now.duration_since(at).as_secs() >= 3)
        {
            self.system_motion = Some((now, crate::motion::system_animations()));
        }
        let system_on = self.system_motion.is_none_or(|(_, on)| on);
        self.collection.art.still = !self.settings.nft_animations(system_on);
        self.a.avatars.still = self.collection.art.still;
        self.collection.ask_unknown = self.settings.ask_unknown_issuers();
        // Pictures off screen for a minute (another screen, scrolled far away) give back their
        // memory.
        let time = ui.input(|i| i.time);
        self.collection.art.forget_unseen(time, 60.0);
        self.a.avatars.forget_unseen(time, 60.0);
        self.touch(ui.ctx());
        let p = self.a.p;
        let mut go = None;
        let mut commands: Vec<Command> = Vec::new();

        let Some(snapshot) = &self.snapshot else {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg))
                .show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(txt(
                            tr!("Connecting to the wallet core…"),
                            14.0,
                            theme::Weight::Regular,
                            p.muted,
                        ))
                    });
                });
            return;
        };
        // Mainnet: a band over every screen, so real money is never taken for test coins.
        if snapshot.network == evolite_api::Network::Mainnet {
            Panel::top("mainnet")
                .exact_size(34.0)
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::new().fill(p.danger).inner_margin(Margin::symmetric(16, 0)))
                .show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(txt(
                            tr!("Mainnet means real DASH. This is a test build of evolite: keep only small amounts here and check the address before every payment."),
                            13.0,
                            theme::Weight::SemiBold,
                            if p.dark { p.bg } else { egui::Color32::WHITE },
                        ));
                    });
                });
        }
        // The data folder is another window's: nothing here but why.
        #[cfg(feature = "dev-tools")]
        let elsewhere = snapshot.elsewhere || self.dev.show_elsewhere;
        #[cfg(not(feature = "dev-tools"))]
        let elsewhere = snapshot.elsewhere;
        if elsewhere {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg))
                .show(ui, |ui| screens::elsewhere(ui, &mut self.a));
            #[cfg(feature = "dev-tools")]
            self.take_screenshot(&ui.ctx().clone());
            return;
        }
        // `--open`: back to the screen asked for, however late the vault opened.
        #[cfg(feature = "dev-tools")]
        let after_unlock = self.dev.shot_screen.unwrap_or(Screen::Overview);
        #[cfg(not(feature = "dev-tools"))]
        let after_unlock = Screen::Overview;
        // Where the vault sends us: no wallet → welcome, closed keys → unlock. Screens with the
        // sidebar need the wallet, so locking from anywhere lands on Unlock.
        self.screen = match (snapshot.vault, self.screen) {
            (VaultState::Locked, Screen::Welcome) => Screen::Unlock,
            (VaultState::Unlocked, Screen::Unlock) => after_unlock,
            (_, Screen::Welcome | Screen::Unlock | Screen::Onboarding(_)) => self.screen,
            // The script check opens from the welcome screen, before any wallet exists.
            #[cfg(feature = "dev-tools")]
            (_, Screen::Probe) => self.screen,
            _ if snapshot.wallet.is_some() => self.screen,
            (VaultState::Locked, _) => Screen::Unlock,
            _ => Screen::Welcome,
        };
        if self.screen == Screen::Unlock && snapshot.vault != VaultState::Locked {
            self.screen = Screen::Welcome;
        }
        #[cfg(feature = "dev-tools")]
        if self.dev.preset_forgot {
            self.screen = Screen::Unlock;
        }
        // the core has made room for the wallet being added: its onboarding opens.
        if snapshot.adding_wallet
            && snapshot.vault == VaultState::NoWallet
            && let Some(flow) = self.adding.take()
        {
            self.onboarding = None;
            self.screen = flow.map_or(Screen::Welcome, Screen::Onboarding);
        }
        // "Delete" on a wallet that was not open: it is open now, at its unlock screen; the
        // deletion asks its own password.
        if let Some((id, forgot)) = &self.delete_other
            && snapshot.vault == VaultState::Locked
            && snapshot.wallets.iter().any(|w| w.open && &w.id == id)
        {
            self.unlock.forgot = Some(if *forgot {
                screens::delete_wallet::DeleteForm::forgot()
            } else {
                screens::delete_wallet::DeleteForm::default()
            });
            self.delete_other = None;
        }
        // A lock (by the button, the timer or the core) takes what was typed or shown with it.
        if keys_gone(self.vault, snapshot.vault) {
            self.send.on_lock();
            self.settings.forget_secrets();
        }
        // A deleted wallet: the new core over the folder knows none of the settings.
        let deleted = snapshot.vault == VaultState::NoWallet
            && self.vault.is_some_and(|v| v != VaultState::NoWallet);
        if deleted {
            for command in self.settings.core_commands() {
                self.core.send(command);
            }
            self.core_told = Some(self.core_settings());
        } else if self.core_told != Some(self.core_settings()) {
            let (fresh, currency) = self.core_settings();
            self.core.send(Command::ReceiveAddress { fresh });
            self.core.send(Command::FiatRate {
                currency: currency.map(str::to_owned),
            });
            self.core_told = Some((fresh, currency));
        }
        crate::format::set_display(self.settings.display(snapshot.fiat.as_ref()));
        if std::mem::take(&mut self.settings.restart) && !crate::is_demo(snapshot) {
            start_again(ui.ctx());
        }
        self.vault = Some(snapshot.vault);
        self.settings.hide_phrase_unless_shown(
            self.screen == Screen::Settings && self.settings.phrase_dialog,
        );
        self.settings
            .close_check_unless_shown(self.screen == Screen::Settings);
        // The identity chosen last time: asked for again once the search has found it, then
        // whatever is shown is the choice to keep.
        if let Some(w) = &snapshot.wallet {
            let shown = w.identity.as_ref().map(|i| i.id.clone());
            if self.identity_restored {
                if w.identities.len() > 1 && shown.is_some() {
                    self.chosen_identity = shown;
                }
            } else if let Some(id) = &self.chosen_identity {
                if w.identities.iter().any(|c| &c.id == id) {
                    if shown.as_ref() != Some(id) {
                        commands.push(Command::ChooseIdentity { id: id.clone() });
                    }
                    self.identity_restored = true;
                } else if w.identity_search == evolite_api::IdentitySearch::Done {
                    // Not among this wallet's identities (another wallet, or gone).
                    self.identity_restored = true;
                }
            } else {
                self.identity_restored = true;
            }
        }
        let mut cx = Ctx::new(snapshot, &mut commands, &mut go);

        if self.screen == Screen::Unlock {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg))
                .show(ui, |ui| {
                    screens::unlock::unlock(
                        ui,
                        &mut self.a,
                        &mut cx,
                        &mut self.unlock,
                        &mut self.settings,
                    );
                });
        } else if matches!(self.screen, Screen::Welcome) {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg))
                .show(ui, |ui| {
                    screens::welcome(ui, &mut self.a, &mut cx, &mut self.settings);
                });
        } else if self.probe_open() {
            // `--screen probe` (dev-tools): no sidebar, there may be no wallet.
            #[cfg(feature = "dev-tools")]
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg).inner_margin(Margin::same(40)))
                .show(ui, |ui| {
                    screens::probe(ui, &mut self.a, &mut cx);
                });
        } else if let Screen::Onboarding(flow) = self.screen {
            let state = match &mut self.onboarding {
                Some(o) if o.flow == flow => o,
                slot => slot.insert(Onboarding::new(flow)),
            };
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg))
                .show(ui, |ui| {
                    match screens::onboarding::onboarding(ui, &mut self.a, &mut cx, state) {
                        Outcome::Stay => {}
                        Outcome::Welcome => {
                            self.settings.nodes = None;
                            cx.go(Screen::Welcome);
                        }
                        Outcome::Wallet => {
                            self.settings.nodes = None;
                            cx.go(Screen::Overview);
                        }
                        // The nodes over the sync, as Settings → Network shows them; the sync
                        // stays where it is.
                        Outcome::Nodes => {
                            self.settings.nodes = Some(screens::settings::NodesTab::Core);
                        }
                    }
                    if let Some(tab) = &mut self.settings.nodes
                        && screens::settings::nodes_dialog(ui, &mut self.a, &cx, tab)
                    {
                        self.settings.nodes = None;
                    }
                    if let Some(i) = state.take_lock_choice() {
                        self.settings.lock_after = i;
                        cx.send(Command::AutoLock {
                            after_secs: self.settings.auto_lock_secs(),
                        });
                    }
                });
        } else {
            let rail = ui.ctx().content_rect().width() < 1100.0;
            let side = if rail { 12 } else { 16 };
            // Right to left (Arabic) `rtl` mirrors the frame: the menu stands on the right.
            Panel::left("sidebar")
                .exact_size(if rail { 76.0 } else { 248.0 })
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    Frame::new()
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.line))
                        .inner_margin(Margin {
                            left: side,
                            right: side,
                            top: 0,
                            bottom: 0,
                        }),
                )
                .show(ui, |ui| {
                    screens::sidebar(
                        ui,
                        &mut self.a,
                        &mut cx,
                        self.screen,
                        rail,
                        &mut self.wallet_menu,
                    );
                });
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg).inner_margin(Margin {
                    left: 40,
                    right: 40,
                    top: 28,
                    bottom: 28,
                }))
                .show(ui, |ui| {
                    // `--screen remind` (dev-tools): the reminder whatever the dates say.
                    #[cfg(feature = "dev-tools")]
                    let remind = self.dev.remind_preset;
                    #[cfg(not(feature = "dev-tools"))]
                    let remind = false;
                    let a = &mut self.a;
                    // Each visit of a screen gets ids of its own: it opens at the top, not
                    // where the last screen was scrolled to (all share one scroll area id
                    // otherwise), and keeps its place while it stays (BUG-48).
                    let visit = self.visits.of(self.screen);
                    ui.push_id(("screen", visit), |ui| match self.screen {
                        Screen::Overview => {
                            egui::ScrollArea::vertical()
                                .auto_shrink(false)
                                .show(ui, |ui| {
                                    screens::settings::check_reminder(
                                        ui,
                                        a,
                                        &mut cx,
                                        &mut self.settings,
                                        remind,
                                    );
                                    screens::overview(ui, a, &mut cx)
                                });
                        }
                        Screen::Activity => {
                            screens::activity(ui, a, &mut cx, &mut self.filter, &mut self.query)
                        }
                        Screen::Send => screens::send::send(
                            ui,
                            a,
                            &mut cx,
                            &mut self.send,
                            self.settings.password_each_send(),
                        ),
                        Screen::Receive => {
                            screens::receive::receive(ui, a, &mut cx, &mut self.receive)
                        }
                        Screen::Identity => {
                            screens::identity::identity(ui, a, &mut cx, &mut self.identity)
                        }
                        Screen::Shielded => {
                            screens::shielded::shielded(ui, a, &mut cx, &mut self.pool)
                        }
                        Screen::Contacts => {
                            screens::contacts::contacts(ui, a, &mut cx, &mut self.contacts)
                        }
                        Screen::Apps => screens::apps::apps(ui, a, &mut cx, &mut self.requests),
                        Screen::Collection => {
                            screens::collection::collection(ui, a, &mut cx, &mut self.collection)
                        }
                        Screen::Settings => {
                            screens::settings::settings(ui, a, &mut cx, &mut self.settings)
                        }
                        Screen::CreateIdentity => screens::create_identity::create_identity(
                            ui,
                            a,
                            &mut cx,
                            &mut self.create_identity,
                        ),
                        // Drawn above, without the sidebar.
                        Screen::Welcome | Screen::Unlock | Screen::Onboarding(_) => {}
                        #[cfg(feature = "dev-tools")]
                        Screen::Probe => {}
                    });
                });
        }
        // "Open "…"" over whatever is on screen (Figma 29).
        if let Some(form) = &mut self.switching
            && screens::switch_wallet::switch_dialog(ui, &mut self.a, &mut cx, form)
        {
            self.switching = None;
        }
        let pay = cx.take_pay();
        if cx.take_top_up() {
            self.identity.open_top_up();
        }
        let wallet_request = cx.take_wallets();
        // "‹ Back to "…"" while a wallet is being added (Figma 31): back to the one it
        // started from.
        if snapshot.adding_wallet && matches!(self.screen, Screen::Welcome | Screen::Onboarding(_))
        {
            let label = match &self.adding_back {
                Some(name) => format!(tr!("Back to “{name}”"), name = crate::i18n::own(name)),
                None => tr!("Go back").to_owned(),
            };
            egui::Area::new(egui::Id::new("cancel-adding"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(32.0, 22.0))
                .show(ui.ctx(), |ui| {
                    if crate::components::text_action(ui, &mut self.a, "chevron-left", &label)
                        .clicked()
                    {
                        commands.push(Command::CancelAddWallet);
                        self.onboarding = None;
                        go = Some(Screen::Welcome);
                    }
                });
            // The rest of Figma 31's top bar: the logo in the middle, the network on the right.
            let width = ui.ctx().content_rect().width();
            let p = self.a.p;
            egui::Area::new(egui::Id::new("adding-logo"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(width / 2.0 - 53.0, 18.0))
                .show(ui.ctx(), |ui| {
                    ui.add(self.a.icons.logo(icons::Logo::Horizontal, &p, 28.0));
                });
            egui::Area::new(egui::Id::new("adding-network"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-32.0, 22.0))
                .show(ui.ctx(), |ui| {
                    crate::components::network_chip(ui, &mut self.a, snapshot.network);
                });
        }

        for command in commands {
            self.core.send(command);
        }
        match wallet_request {
            Some(crate::context::WalletRequest::Add(flow)) => {
                self.core.send(Command::AddWallet);
                self.adding = Some(flow);
                self.adding_back = snapshot
                    .wallets
                    .iter()
                    .find(|w| w.open)
                    .map(|w| w.name.clone());
            }
            Some(crate::context::WalletRequest::Manage) => {
                self.settings.section = Section::Wallets;
                go = Some(Screen::Settings);
            }
            Some(crate::context::WalletRequest::DeleteOther { id, forgot }) => {
                self.core.send(Command::SwitchWallet {
                    id: id.clone(),
                    password: None,
                });
                self.delete_other = Some((id, forgot));
            }
            Some(crate::context::WalletRequest::Open(id)) => {
                self.switching = Some(screens::switch_wallet::SwitchForm::new(id));
            }
            None => {}
        }
        // "Send" on a contact: the Send screen, from Core, to that name.
        if let Some(to) = self.contacts.pay.take() {
            self.send.replace(screens::send::SendForm {
                recipient: to,
                ..screens::send::SendForm::empty()
            });
            go = Some(Screen::Send);
        }
        // "Withdraw to Core", "Transfer", "Send again": the Send screen from that source,
        // to that recipient (and that amount).
        if let Some(pay) = pay {
            self.send
                .replace(screens::send::SendForm::paying(pay.source, pay.to));
            if let Some(amount) = pay.amount {
                self.send.amount = crate::format::input_text(amount);
            }
        }
        if let Some(screen) = go {
            if !matches!(screen, Screen::Onboarding(_)) {
                // Leaving the onboarding forgets what was typed there.
                self.onboarding = None;
            }
            self.screen = screen;
        }
        // Settings → Appearance picks the theme; "As in the system" follows the OS.
        let dark = match self.settings.theme {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::System => ui.ctx().system_theme() != Some(egui::Theme::Light),
        };
        if dark != p.dark {
            self.a.p = if dark { theme::DARK } else { theme::LIGHT };
            theme::apply_visuals(ui.ctx(), &self.a.p);
        }
        self.watch
            .screen(crate::diagnostics::screen_name(self.screen));
        self.watch.frame(frame_started.elapsed());
        // A screen with the phrase on it keeps the window out of screenshots and recordings
        // from this very frame; the window comes back once no such screen is drawn.
        let hide = crate::capture::take();
        if hide != self.capture_hidden && (crate::capture::apply(frame, hide) || !hide) {
            self.capture_hidden = hide;
        }
        #[cfg(feature = "dev-tools")]
        self.take_screenshot(&ui.ctx().clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deletion_takes_what_was_typed_as_a_lock_does() {
        use VaultState::*;
        assert!(keys_gone(Some(Unlocked), Locked));
        // "Delete wallet" goes from open straight to no wallet.
        assert!(keys_gone(Some(Unlocked), NoWallet));
        assert!(!keys_gone(Some(Unlocked), Unlocked));
        assert!(!keys_gone(Some(Locked), NoWallet));
        assert!(!keys_gone(None, Locked));
    }

    /// BUG-48: History scrolled down, then Identity → Identity's scroll area is a new one
    /// (opens at the top); staying on a screen keeps it; coming back is a new visit again.
    #[test]
    fn every_visit_of_a_screen_scrolls_on_its_own() {
        let mut visits = Visits::default();
        let history = visits.of(Screen::Activity);
        assert_eq!(
            visits.of(Screen::Activity),
            history,
            "staying keeps the place"
        );
        let identity = visits.of(Screen::Identity);
        assert_ne!(identity, history);
        assert_eq!(visits.of(Screen::Identity), identity);
        let back = visits.of(Screen::Activity);
        assert_ne!(back, history, "coming back opens at the top");
        assert_ne!(back, identity);
    }

    /// BUG-48, end to end in egui: two places under different visits keep separate scroll
    /// offsets, though the scroll areas themselves have no id of their own.
    #[test]
    fn a_scrolled_screen_does_not_scroll_the_next_one() {
        let ctx = egui::Context::default();
        let offset = |visit: u32, scroll_to: Option<f32>| {
            let mut seen = 0.0;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.push_id(("screen", visit), |ui| {
                    let mut area = egui::ScrollArea::vertical().max_height(100.0);
                    if let Some(y) = scroll_to {
                        area = area.vertical_scroll_offset(y);
                    }
                    seen = area
                        .show(ui, |ui| ui.allocate_space(egui::vec2(10.0, 1000.0)))
                        .state
                        .offset
                        .y;
                });
            });
            out.textures_delta.clear();
            seen
        };
        assert_eq!(offset(1, Some(500.0)), 500.0);
        assert_eq!(
            offset(1, None),
            500.0,
            "the scrolled screen keeps its place"
        );
        assert_eq!(offset(2, None), 0.0, "the next screen opens at the top");
    }
}
