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
    /// `--screen overview-syncing` / `send-syncing`: the screen while the sync still runs.
    syncing_preset: bool,
}

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
            syncing_preset: dev.preset("overview-syncing") || dev.preset("send-syncing"),
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
                locked: Some(31_000_000),
                name: None,
            });
        }
        // `--screen overview-syncing` / `send-syncing`: in the middle of the block filters, as a
        // restored wallet on testnet is for minutes.
        if self.dev.syncing_preset
            && let Some(s) = self.snapshot.as_mut()
        {
            s.sync.stage = evolite_api::SyncStage::Filters;
            s.sync.progress = 0.58;
            s.sync.chainlocked = false;
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
        // `--screen send-confirm`: the last look asks where the payment goes, as a click does.
        if preset("send-confirm") {
            self.core.send(Command::PreviewPayment {
                to: "@alice".into(),
            });
        }
        if demo {
            // `--screen send-confirm` opens the last look before signing.
            self.send.confirming = preset("send-confirm");
            // `--screen send-contact`: a name that is not a contact yet (21).
            // `--screen send-book`: nothing typed yet, the address book offered.
            if preset("send-contact") {
                self.send.recipient = "@carol".into();
            } else if preset("send-book") {
                self.send.recipient.clear();
            }
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
                unshielding: unshield,
                unshield_to_identity: preset("unshield-identity"),
                unshield_amount: if unshield && !real_shot {
                    "0,3000".into()
                } else {
                    String::new()
                },
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
        } else if preset("change-password") || preset("delete") || preset("security") {
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
        if preset("create-identity") || preset("creating") {
            self.create_identity.name = "evolite-2026".into();
        }
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
        if preset("delete") {
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
        let demo_other = "5e6f7a8b";
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
