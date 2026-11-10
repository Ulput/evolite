//! The `dev-tools` part of the app: screens opened in a Figma state (`--screen`), the real
//! wallet looked at without changes (`--open`), and the screenshot that ends the run.

use super::App;
use crate::cli::DevLaunch;
use crate::screens;
use crate::screens::Screen;
use crate::screens::settings::Section;
use evolite_api::{Command, VaultState};

/// What the screenshot tools keep between frames.
pub(super) struct DevState {
    /// `--screenshot <file.png>`: save the window after a few frames and quit (for checks on
    /// other systems, where there is no way to capture the window from outside).
    screenshot: Option<String>,
    frames: u32,
    /// When the picture is due and whether it was asked for (`--open` waits for the wallet).
    shot_wait: crate::shot::ShotWait,
    started: std::time::Instant,
    /// `--open`: the shot waits for the real wallet (synced, open) and then a few seconds more.
    pub(super) real_shot: bool,
    /// `--open` asked the core to open the vault: the shot waits for it.
    shot_unlock: bool,
    /// `--open`: the screen to show once the vault is open.
    pub(super) shot_screen: Option<Screen>,
    /// `--open activity-card` / `edit-profile`: opened from the real wallet once it is in.
    real_preset: Option<&'static str>,
    /// `--screen elsewhere`: the "open in another window" screen, for screenshots.
    pub(super) show_elsewhere: bool,
    /// `--screen forgot`: the unlock screen with "Forgot the password?", though the demo is open.
    pub(super) preset_forgot: bool,
    /// `--screen remind`: Overview with the phrase-check reminder.
    pub(super) remind_preset: bool,
    /// `--screen identity-unfinished`: Identity with a creation stopped after the Core lock
    /// (Figma "Screens", the Arabic frames), which the example wallet does not have.
    unfinished_preset: bool,
    /// `--screen create-identity-short` / `create-identity-unchecked`: Core has too little for
    /// the identity, or was last checked a while ago (FEAT-0014, frames 9 and 10).
    core_preset: Option<CorePreset>,
    /// `--screen identity-created`: the identity shown is not the phrase's first (R-9).
    second_identity_preset: bool,
    /// `--screen top-up-stopped` / `top-up-awaiting`: Identity with a cut-short top-up, to
    /// continue or (Platform already took it) awaiting its ChainLock (BUG-7).
    top_up_preset: Option<bool>,
    /// `--screen overview-syncing` / `send-syncing`: the screen while the sync still runs.
    syncing_preset: bool,
    /// `--screen send-pool-unread` / `send-pool-unknown`: Send from the pool before its first read
    /// in this run, with the amount kept from before (`true`) or none (BUG-113).
    pool_preset: Option<bool>,
    /// `--screen activity-notes-unreadable`: the notes file did not open (FEAT-0003, E2).
    pub(super) notes_error: Option<evolite_api::NotesError>,
    /// Tests only: every Core card says that part of its details was not saved (FEAT-0005a, FR-6).
    pub(super) incomplete_details: bool,
    /// The unlock presets of FEAT-0002: the wallets' wrong passwords as the core would send them.
    pause_preset: Option<PausePreset>,
    /// The clock the screens read, stopped at the start: a countdown in a screenshot shows
    /// the time its preset names (designer, Q-D5).
    pub(super) frozen_now: Option<chrono::DateTime<chrono::Local>>,
}

/// `--screen unlock`, `unlock-warn`, `unlock-wait`, `unlock-wait-end` (one wallet),
/// `wallet-unlock-wait` and `wallet-switch-wait` (the demo's three, "Savings" in a pause).
struct PausePreset {
    /// «Welcome back»: only the open wallet is listed.
    one_wallet: bool,
    /// The wallet in the state below; `None`: the open one.
    wallet: Option<&'static str>,
    /// Wrong passwords in a row, and the seconds of the pause left; `None`: the demo core's own.
    tries: Option<(u32, Option<i64>)>,
}

impl PausePreset {
    fn of(dev: &DevLaunch) -> Option<Self> {
        let one = |tries| {
            Some(Self {
                one_wallet: true,
                wallet: None,
                tries,
            })
        };
        let savings = Some(Self {
            one_wallet: false,
            wallet: Some(DEMO_OTHER),
            // The 8th wrong password: a pause of 5 min, 4:12 of it left (mockups 3 and 4).
            tries: Some((8, Some(252))),
        });
        if dev.preset("unlock") {
            one(None)
        } else if dev.preset("unlock-warn") {
            one(Some((4, None)))
        } else if dev.preset("unlock-wait") {
            one(Some((6, Some(45))))
        } else if dev.preset("unlock-wait-end") {
            one(Some((5, None)))
        } else if dev.preset("wallet-unlock-wait") || dev.preset("wallet-switch-wait") {
            savings
        } else {
            None
        }
    }
}

/// What `--screen create-identity-short` / `create-identity-unchecked` do to Core.
#[derive(Clone, Copy)]
enum CorePreset {
    /// 0.15 DASH: less than the identity takes (frame 9).
    Short,
    /// Last checked 20 minutes ago, the sync not there yet (frame 10).
    Unchecked,
}

/// The demo's second wallet ("Savings").
const DEMO_OTHER: &str = "5e6f7a8b";

impl DevState {
    pub(super) fn new(dev: &DevLaunch, screen: Screen) -> Self {
        let real_preset = ["activity-card", "edit-profile", "contact-profile"]
            .into_iter()
            .find(|name| dev.preset(name) && dev.real_shot);
        Self {
            screenshot: dev.screenshot.clone(),
            frames: 0,
            shot_wait: Default::default(),
            started: std::time::Instant::now(),
            real_shot: dev.real_shot,
            shot_unlock: false,
            shot_screen: dev.real_shot.then_some(screen),
            real_preset,
            show_elsewhere: dev.preset("elsewhere"),
            preset_forgot: dev.preset("forgot"),
            remind_preset: dev.preset("remind"),
            unfinished_preset: dev.preset("identity-unfinished"),
            core_preset: if dev.preset("create-identity-short") {
                Some(CorePreset::Short)
            } else if dev.preset("create-identity-unchecked") {
                Some(CorePreset::Unchecked)
            } else {
                None
            },
            second_identity_preset: dev.preset("identity-created"),
            top_up_preset: if dev.preset("top-up-stopped") {
                Some(false)
            } else if dev.preset("top-up-awaiting") {
                Some(true)
            } else {
                None
            },
            syncing_preset: dev.preset("overview-syncing") || dev.preset("send-syncing"),
            pool_preset: if dev.preset("send-pool-unread") {
                Some(true)
            } else if dev.preset("send-pool-unknown") {
                Some(false)
            } else {
                None
            },
            incomplete_details: false,
            notes_error: dev
                .preset("activity-notes-unreadable")
                .then_some(evolite_api::NotesError::PutAside),
            frozen_now: PausePreset::of(dev)
                .filter(|p| p.tries.is_some())
                .map(|_| chrono::Local::now()),
            pause_preset: PausePreset::of(dev),
        }
    }
}

impl App {
    /// `--open`: the real wallet is open and synced (or there is no wallet to open), and has
    /// been for `EVOLITE_SHOT_SETTLE` seconds (15 by default) — names, pictures and balances come
    /// in after the sync; at the latest 300 s past that ([`crate::shot::ShotWait`]).
    fn real_ready(&mut self) -> bool {
        let settled = self.snapshot.as_ref().is_some_and(|s| {
            s.sync.stage == evolite_api::SyncStage::Done
                && (s.vault == VaultState::Unlocked || !self.dev.shot_unlock)
        });
        // `EVOLITE_SHOT_SETTLE`: seconds to wait after the sync, for what comes later (the pool).
        let settle = std::env::var("EVOLITE_SHOT_SETTLE")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(15);
        self.dev.shot_wait.ready(
            settled,
            self.dev.started,
            std::time::Duration::from_secs(settle),
            std::time::Instant::now(),
        )
    }

    pub(super) fn take_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.dev.screenshot.clone() else {
            return;
        };
        self.dev.frames += 1;
        ctx.request_repaint();
        // The picture first, whatever this frame decides next: a frame that returned before
        // looking lost it, and the app waited for it forever.
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            let [w, h] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            image::save_buffer(&path, &rgba, w as u32, h as u32, image::ColorType::Rgba8)
                .expect("save screenshot");
            println!("screenshot saved to {path} ({w} x {h})");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // Images (SVG) load asynchronously and the core answers in its own time; NFT pictures
        // are drawn by the renderer, up to several seconds each.
        // With `--open` the pictures start after the sync: a minute from then.
        let drawing_since = if self.dev.real_shot {
            self.dev.shot_wait.since(self.dev.started)
        } else {
            self.dev.started
        };
        let limit = if self.dev.real_shot { 60 } else { 30 };
        let drawing = (self.collection.art.busy() || self.a.avatars.busy())
            && drawing_since.elapsed().as_secs() < limit;
        if self.dev.real_shot && !self.real_ready() {
            return;
        }
        if self.dev.frames >= 40 && !drawing && self.dev.shot_wait.ask(std::time::Instant::now()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
    }

    /// A new snapshot: `--screen identity-unfinished` gives it a stopped creation, and `--open`
    /// opens what it asked for once the real wallet has it.
    pub(super) fn dev_snapshot(&mut self, ctx: &egui::Context) {
        if self.dev.unfinished_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            w.unfinished_identity = Some(evolite_api::UnfinishedIdentity {
                identity: None,
                locked: Some(31_000_000),
                name: None,
            });
        }
        if let Some(core) = self.dev.core_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            match core {
                CorePreset::Short => w.balances.core = 15_000_000,
                CorePreset::Unchecked => {
                    let at = chrono::Local::now().timestamp() - 20 * 60;
                    w.balances.core_as_of = Some(evolite_api::CoreAsOf::Checked(at));
                }
            }
        }
        if self.dev.second_identity_preset
            && let Some(id) = self
                .snapshot
                .as_mut()
                .and_then(|s| s.wallet.as_mut())
                .and_then(|w| w.identity.as_mut())
        {
            id.slot = 1;
        }
        if let Some(awaiting_chain_lock) = self.dev.top_up_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            w.unfinished_top_up = Some(evolite_api::UnfinishedTopUp {
                locked: 50_000_000,
                awaiting_chain_lock,
            });
        }
        // `--screen overview-syncing` / `send-syncing`: in the middle of the block filters, as a
        // restored wallet on testnet is for minutes.
        if self.dev.syncing_preset
            && let Some(s) = self.snapshot.as_mut()
        {
            s.sync.stage = evolite_api::SyncStage::Filters;
            s.sync.progress = Some(0.58);
            s.sync.chainlocked = false;
        }
        // `--screen send-pool-unread` / `send-pool-unknown`: the pool not read in this run yet, with
        // the demo's amount kept from before or none (BUG-113).
        if let Some(kept) = self.dev.pool_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            w.pool.checked_at = None;
            if !kept {
                w.balances.shielded = 0;
            }
        }
        if let Some(e) = self.dev.notes_error
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            w.notes_error = Some(e);
        }
        if self.dev.incomplete_details
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            for d in w.activity.iter_mut().filter_map(|a| a.detail.as_mut()) {
                d.incomplete = true;
            }
        }
        if let Some(pp) = &self.dev.pause_preset
            && let Some(s) = self.snapshot.as_mut()
        {
            if pp.one_wallet {
                s.wallets.retain(|w| w.open);
            }
            let now = self
                .dev
                .frozen_now
                .unwrap_or_else(chrono::Local::now)
                .timestamp();
            if let Some((wrong, left)) = pp.tries
                && let Some(w) = s
                    .wallets
                    .iter_mut()
                    .find(|w| pp.wallet.map_or(w.open, |id| w.id == id))
            {
                w.wrong_tries = wrong;
                w.wait_until = left.map(|left| now + left);
            }
        }
        self.open_real_preset(ctx);
    }

    /// `--open activity-card`, `edit-profile` and `contact-profile`: the newest operation's card,
    /// the profile form of the shown identity and the first contact's profile, as a click opens
    /// them, once the real wallet has them.
    fn open_real_preset(&mut self, ctx: &egui::Context) {
        let Some(wallet) = self.snapshot.as_ref().and_then(|s| s.wallet.as_ref()) else {
            return;
        };
        match self.dev.real_preset {
            Some("activity-card") if !wallet.activity.is_empty() => {
                screens::open_card(ctx, wallet.activity[0].id);
            }
            Some("contact-profile") if !wallet.contacts.is_empty() => {
                self.contacts.profile = Some(wallet.contacts[0].identity_id.clone());
            }
            Some("edit-profile") if wallet.identity.is_some() => {
                let profile = wallet.identity.as_ref().and_then(|i| i.profile.as_ref());
                self.identity.editing = Some(screens::identity::ProfileForm::from(profile));
            }
            _ => return,
        }
        self.dev.real_preset = None;
    }

    /// `--screen <name>`: the screen's forms and dialogs in their Figma state, before the
    /// settings saved in the data folder are read (`--open` reads them too).
    pub(super) fn apply_presets(&mut self, ctx: &egui::Context, dev: &DevLaunch, demo: bool) {
        let real_shot = dev.real_shot;
        let preset = |name: &str| dev.preset(name);
        if let Some(link) = dev.example_link {
            self.core.send(Command::OpenAppLink { text: link.into() });
        }
        // `--screen activity-card`: History with an operation's card open (24).
        if preset("activity-card") && !real_shot {
            screens::open_card(ctx, 3);
        }
        // `--screen activity-filters`: History with «Filters» open on «In» and «30 days»;
        // `activity-filters-empty`: «Pending» and «Names», which leave nothing (FEAT-0009).
        if preset("activity-filters") {
            let f = &mut self.history.filters;
            f.flow = Some(evolite_api::Flow::In);
            f.period = screens::activity_filters::Period::Days(30);
            self.history.open = true;
        }
        if preset("activity-filters-empty") {
            let f = &mut self.history.filters;
            f.status = Some(screens::activity_filters::Status::Pending);
            f.kinds = screens::activity_filters::kind::NAMES;
        }
        // FEAT-0005a: a transfer between own addresses (mockup 01) and a payment in.
        if preset("activity-card-self") && !real_shot {
            screens::open_card(ctx, 17);
        }
        if preset("activity-card-received") && !real_shot {
            screens::open_card(ctx, 8);
        }
        // FEAT-0005b: the demo's Platform lines (mockups 03-08).
        for (name, line) in [
            ("activity-card-name", 13),
            ("activity-card-withdrawal-waiting", 18),
            ("activity-card-withdrawal-paid", 12),
            ("activity-card-payout", 19),
            ("activity-card-lock-not-added", 20),
            ("activity-card-shield", 6),
            ("activity-card-old", 14),
            ("activity-card-pool-sent", 21),
            ("activity-card-pool-received", 22),
            ("activity-card-top-up", 4),
            ("activity-card-lock-used", 23),
            ("activity-card-lock-partly", 24),
            ("activity-card-lock-not-received", 25),
            ("activity-card-lock-unknown", 10),
        ] {
            if preset(name) && !real_shot {
                screens::open_card(ctx, line);
            }
        }
        // FEAT-0003: the notes in History and in an operation's card (Figma 05, 24; the designer's
        // frames 2, 4-7). The demo's note is the core's example, in the interface's language.
        if !real_shot {
            let note = crate::i18n::t_in(
                crate::i18n::current(),
                tr!("For the logo design · invoice no. 14 of 30 October"),
            );
            let edited = preset("activity-card-note-edit");
            let failed = preset("activity-card-note-error");
            if preset("activity-card-note") || edited || failed {
                screens::open_card(ctx, 3);
            }
            if preset("activity-card-add-note") {
                screens::open_card(ctx, 2);
            }
            if edited {
                screens::activity::edit_note(ctx, "demo:3", &note);
            }
            if failed {
                screens::activity::note_failed_example(ctx, "demo:3", &note);
            }
            // A word of the demo's note.
            if preset("activity-search-note") {
                self.history.query =
                    crate::i18n::t_in(crate::i18n::current(), tr!("logo")).into_owned();
            }
        }
        // `--screen send-confirm`: the last look asks where the payment goes, as a click does.
        if preset("send-confirm") || preset("send-confirm-note") || preset("send-confirm-link") {
            self.core.send(Command::PreviewPayment {
                to: "@alice".into(),
            });
        }
        if demo {
            // `--screen send-confirm` opens the last look before signing.
            self.send.confirming = preset("send-confirm")
                || preset("send-confirm-note")
                || preset("send-confirm-link");
            // FEAT-0006: what a QR leaves in the form (Figma 02, 04, 09).
            const LINK: &str =
                "dash:yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f?amount=0.25&label=Invoice%2014";
            if preset("send-qr-filled") {
                self.send.take_link(LINK);
                // The frame sets how amounts are written after this: shown as the example is.
                self.send.suggested = crate::format::parse_amount(&self.send.amount);
            } else if preset("send-confirm-link") {
                self.send.take_link(
                    "dash:yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f?user=alice&label=Invoice%2014",
                );
            } else if preset("send-qr-error") {
                self.send.recipient.clear();
                self.send.take_scan(&crate::qr_scan::Scanned::NoQr);
            }
            // `--screen send-note`, `send-confirm-note`: a note typed (Figma 03); `send-note-limit`:
            // more pasted than a note holds, cut and said so (the designer's frame 8).
            if preset("send-note") || preset("send-confirm-note") {
                self.send.type_note(&crate::i18n::t_in(
                    crate::i18n::current(),
                    tr!("For the logo design"),
                ));
            } else if preset("send-note-limit") {
                let long = crate::i18n::t_in(
                    crate::i18n::current(),
                    tr!(
                        "For the logo, business cards and letterhead design: invoice no. 14 of 30 October, the second stage under contract no. 7, the advance received, the rest by 15 November"
                    ),
                );
                self.send.type_note(&format!("{long} {long}"));
            }
            // `--screen send-contact`: a name that is not a contact yet (21).
            // `--screen send-book`: nothing typed yet, the address book offered.
            if preset("send-contact") {
                self.send.recipient = "@carol".into();
            } else if preset("send-book") {
                self.send.recipient.clear();
            }
            // `--screen send-pool-unread` / `send-pool-unknown`: from the pool, to an address: the card,
            // the line under the row, «Left» and the button (BUG-113).
            if preset("send-pool-unread") || preset("send-pool-unknown") {
                self.send.source = 2;
                self.send.recipient = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f".into();
            }
        }
        // `--screen send-qr-menu` (`--open send-qr-menu` over the real wallet): «To»'s QR menu
        // open, «From the clipboard» active — not the clipboard's own answer, so that the shot
        // is the same on every machine. Nothing is read until an item is chosen.
        if preset("send-qr-menu") {
            self.send.scanner.menu = true;
            self.send.scanner.clip_picture = Some(true);
        }
        // `--screen edit-profile` opens 23 over Identity.
        if preset("edit-profile") && !real_shot {
            self.identity.editing = Some(screens::identity::ProfileForm {
                display: "Satoshi Nakamoto".into(),
                about: tr!("I write about Dash Platform and collect generative NFTs. Pay through a DashPay contact — that way each payment goes to a new address.").into(),
                avatar_url: "https://ipfs.io/ipfs/bafybeigdyrz/satoshi.png".into(),
                ..Default::default()
            });
        }
        // `--screen top-up` opens "Top up the identity" over Identity.
        self.identity.top_up = preset("top-up").then(screens::identity::TopUpForm::default);
        // `--screen add-name` opens "Add a name" with a name being checked.
        self.identity.adding_name = preset("add-name").then(|| screens::identity::NameForm {
            name: "satoshi-nakamoto-2026".into(),
            ..Default::default()
        });
        // `--screen add-contact` opens 20 with @carol, `--screen contact-profile` opens 22.
        self.contacts.adding = preset("add-contact").then(|| screens::contacts::AddContact {
            name: "@carol".into(),
            ..Default::default()
        });
        self.contacts.my_qr = preset("my-qr");
        if preset("contact-profile") && !real_shot {
            self.contacts.profile = Some("3Ah7Kp2Qx9Lm4Vt8Rn5Wc1Jd6Yb2Fs7Ge4Hu3Mz9Lp1".into());
        }
        // `--screen shield` opens 09 with the proof part-way, as in Figma.
        // With `--open` the dialog opens empty: no made-up proof under way.
        self.pool = if preset("shield") && !real_shot {
            screens::shielded::PoolState {
                shielding: true,
                progress: Some(0.42),
                ..Default::default()
            }
        } else {
            // `--screen unshield` and `--screen pool-address`: the other two dialogs.
            let unshield = preset("unshield") || preset("unshield-identity");
            screens::shielded::PoolState {
                // `--screen shield-core` and `--screen unshield-identity`: the Core source and
                // the identity destination; `--open shield` is the empty dialog over the real core.
                shielding: preset("shield-core") || preset("shield"),
                from: if preset("shield-core") { 2 } else { 0 },
                // An example amount only in `--screen shield-core`: the dialog opens empty.
                amount: if preset("shield-core") && !real_shot {
                    crate::format::input_text(50_000_000)
                } else {
                    String::new()
                },
                suggested: (preset("shield-core") && !real_shot).then_some(50_000_000),
                unshielding: unshield,
                unshield_to_identity: preset("unshield-identity"),
                unshield_amount: if unshield && !real_shot {
                    crate::format::input_text(30_000_000)
                } else {
                    String::new()
                },
                unshield_suggested: (unshield && !real_shot).then_some(30_000_000),
                address_open: preset("pool-address"),
                ..Default::default()
            }
        };
        // `--screen gift` opens the collection with the gift dialog.
        self.collection.gifting = preset("gift");
        // `--screen phrase` opens 17 over Backup, `--screen appearance` that section.
        let report = preset("report") || preset("report-letter");
        self.settings.section = if preset("phrase") || preset("check-phrase") || preset("backup") {
            Section::Backup
        } else if preset("appearance") || preset("appearance-rate") || preset("language") {
            Section::Appearance
        } else if preset("change-password")
            || preset("delete")
            || preset("delete-notes")
            || preset("security")
        {
            Section::Security
        } else if preset("about") || preset("about-end") || report {
            Section::About
        } else if preset("notifications") {
            Section::Notifications
        } else if preset("wallets") || preset("wallet-rename") || preset("wallet-more") {
            Section::Wallets
        } else {
            Section::Network
        };
        self.settings.phrase_dialog = preset("phrase");
        // `--screen create-identity` shows a name, as in Figma; `--screen creating` the
        // same while the core is on step 2.
        if preset("create-identity") || preset("creating") || preset("create-identity-confirm") {
            self.create_identity.name = "evolite-2026".into();
        }
        // `--screen create-identity-confirm`: «Check the new identity» over it (FEAT-0014,
        // frame 7); `create-identity-nameless`, `-short`, `-unchecked` keep the name empty.
        self.create_identity.confirming = preset("create-identity-confirm");
        // `--screen identity-created`: the banner after a creation (frame 5).
        self.identity.created =
            preset("identity-created").then_some(screens::identity::Created::Awaiting(None));
        self.create_identity.waiting = preset("creating");
        self.create_identity.step = preset("creating").then_some(2);
        self.wallet_menu = preset("wallet-menu");
        // `--screen check-phrase` opens "Check the phrase" over Backup with the words asked.
        if preset("check-phrase") {
            self.settings.check = Some(screens::settings::PhraseCheckForm {
                password: zeroize::Zeroizing::new(if real_shot {
                    String::new()
                } else {
                    "demo".into()
                }),
                ask_on_open: !real_shot,
                ..Default::default()
            });
        }
        // `--screen change-password` opens that dialog over Security.
        if preset("change-password") {
            self.settings.change = Some(Default::default());
        }
        // `--screen delete` and `--screen forgot`: the deletion dialogs, for screenshots.
        // `--screen delete-notes`: the same, for its block about the notes (the demo has notes).
        if preset("delete") || preset("delete-notes") {
            self.settings.delete = Some(Default::default());
        }
        // `--screen report`: "Report a problem" filled in, for screenshots.
        if report {
            self.settings.report = Some(screens::settings::report::ReportForm {
                summary: tr!("A payment from the Platform address does not go through").into(),
                steps: tr!("Opened “Send”, chose “Platform address”, pasted a Core address, pressed “Review and send”.").into(),
                expected: tr!("The payment review window.").into(),
                actual: tr!("Nothing happens, the button becomes active again.").into(),
                often: 2,
                letter: preset("report-letter"),
                ..Default::default()
            });
        }
        // `--screen about-end`: About scrolled to the journal.
        self.settings.scroll_to_end = preset("about-end").then_some(0.0);
        if preset("resync") {
            self.settings.resync = Some(Default::default());
        }
        if self.dev.preset_forgot {
            self.unlock.forgot = Some(screens::delete_wallet::DeleteForm::forgot());
        }
        // For screenshots of the demo's three wallets (Figma 28–32): the unlock screen's
        // choice (the same forced screen as `forgot`, without its dialog), "Open "…"", the
        // name and colour, the onboarding of another wallet, a wallet's "…".
        let demo_other = DEMO_OTHER;
        // FEAT-0002: the unlock screen of one wallet or three; the typing kept in a pause.
        if self.dev.pause_preset.is_some() && !preset("wallet-switch-wait") {
            self.dev.preset_forgot = true;
        }
        if preset("unlock-wait") || preset("unlock-wait-end") {
            self.unlock.password = zeroize::Zeroizing::new("main-wallet-26".into());
        }
        if preset("unlock-warn") {
            self.unlock.error = Some(tr!("Wrong password").into());
        }
        if preset("wallet-unlock-wait") {
            self.unlock.chosen = Some(demo_other.into());
        }
        if preset("wallet-switch-wait") {
            self.switching = Some(screens::switch_wallet::SwitchForm::new(demo_other.into()));
        }
        if preset("wallet-unlock") {
            self.dev.preset_forgot = true;
            self.unlock.password = zeroize::Zeroizing::new("main-wallet-26".into());
        }
        if preset("wallet-switch") {
            let mut form = screens::switch_wallet::SwitchForm::new(demo_other.into());
            form.password = zeroize::Zeroizing::new("savings-26".into());
            self.switching = Some(form);
        }
        if preset("wallet-rename") {
            self.settings.rename = Some(screens::settings::RenameForm {
                id: demo_other.into(),
                name: tr!("Savings").into(),
                color: 1,
                ..Default::default()
            });
        }
        if preset("wallet-more") {
            self.settings.wallet_more = Some(demo_other.into());
        }
        if preset("wallet-add") {
            self.core.send(Command::AddWallet);
            self.adding = Some(None);
            self.adding_back = Some(tr!("Main").into());
        }
    }

    /// What the settings change elsewhere, over the settings read from the data folder: hidden
    /// amounts, another unit and format with the rate, the language picker, the own-node form,
    /// the node lists.
    pub(super) fn apply_settings_presets(&mut self, dev: &DevLaunch) {
        if dev.preset("hidden") {
            self.settings.set("hide-amounts", true);
        }
        // `--screen language`: Appearance with the language picker open.
        self.settings.lang_picker = dev.preset("language");
        if dev.preset("units") || dev.preset("appearance-rate") {
            self.settings.unit = 1;
            self.settings.number_style = 2;
            self.settings.set("fiat-rate", true);
        }
        if dev.preset("own-node") {
            self.settings.node = Some(screens::settings::NodeForm {
                address: "192.168.1.10:19999".into(),
                ..Default::default()
            });
        }
        self.settings.nodes = [
            ("nodes", screens::settings::NodesTab::Core),
            ("nodes-dapi", screens::settings::NodesTab::Platform),
            ("sync-nodes", screens::settings::NodesTab::Core),
        ]
        .into_iter()
        .find(|(name, _)| dev.preset(name))
        .map(|(_, tab)| tab);
    }

    /// `--open`: the vault opens with `EVOLITE_SHOT_PASSWORD` from the environment (never an
    /// argument, which other programs can read).
    pub(super) fn unlock_for_shot(&mut self) {
        if self.dev.real_shot
            && let Some(password) = std::env::var_os("EVOLITE_SHOT_PASSWORD")
        {
            self.core.send(Command::Unlock {
                password: evolite_api::Password::new(password.to_string_lossy().into_owned()),
            });
            self.dev.shot_unlock = true;
        }
    }
}
