//! The `dev-tools` part of the app: screens opened in a Figma state (`--screen`), the real
//! wallet looked at without changes (`--open`), and the screenshot that ends the run.

use super::App;
use crate::cli::DevLaunch;
use crate::screens;
use crate::screens::Screen;
use crate::screens::settings::Section;
use evolite_api::{Command, VaultState};

/// `--screen fund-address-recovered`: where «Continue» of a lock found after a restore goes —
/// another of the wallet's Platform addresses than the one signed for (QA's C2, BUG-171).
/// BUG-265: the presets of the 10 long-named identities, one per screen that shows them.
pub(crate) const LONG_IDENTITIES: [&str; 4] = [
    "identities-long",
    "tokens-identities-long",
    "collection-identities-long",
    "workshop-identities-long",
];

/// 10 identities with long names, `first` (the shown one) first with its id, the last without a
/// name: werner555's wallet (BUG-265).
pub(crate) fn long_identities(
    first: &evolite_api::IdentityChoice,
) -> Vec<evolite_api::IdentityChoice> {
    let mut all = vec![evolite_api::IdentityChoice {
        name: Some("werner555-the-first-identity-with-a-very-long-name".into()),
        ..first.clone()
    }];
    for i in 1..10u32 {
        all.push(evolite_api::IdentityChoice {
            id: format!("{i}Hk2Wm4nPq8rSt3uVw5xYz6aBc9dEf2gHj7kLm1n9QwP"),
            name: (i != 9).then(|| format!("werner555-identity-number-{i}-with-a-long-name")),
            slot: i,
        });
    }
    all
}

pub(super) fn recovered_to() -> String {
    on_this_network("kpmx2t8gs39wnd5v7n4k3l2qx9y8w6e5r4t3d2wd62z")
}

/// A preset's Platform address on the network the app runs on: `rest` after its prefix (BUG-211).
fn on_this_network(rest: &str) -> String {
    format!(
        "{}{rest}",
        evolite_api::address::hrp(crate::context::network())
    )
}

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
    /// `--screen names-offline` (FEAT-0039 AC-A4): the network never answers the name
    /// suggestions — their asks do not reach the core.
    pub(super) names_offline: bool,
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
    /// `--screen activity-card-stuck` (FEAT-0046): the payment of line 3 not confirmed by the
    /// network, until it is broadcast again.
    stuck_preset: bool,
    /// `--screen identity-created`: the identity shown is not the phrase's first (R-9).
    second_identity_preset: bool,
    /// `--screen *-identities-long` (BUG-265, TEAM.md §8d: a hard preset): 10 identities with
    /// long names, one without a name — werner555's wallet.
    long_identities_preset: bool,
    /// `--screen top-up-stopped` / `top-up-awaiting`: Identity with a cut-short top-up, to
    /// continue or (Platform already took it) awaiting its ChainLock (BUG-7).
    top_up_preset: Option<bool>,
    /// `--screen fund-address-unfinished` / `-resume` / `-steps`: a funding of the Platform address whose
    /// lock is made and not taken yet (FEAT-0015 stage 2, frames 23, 24, 24b).
    pub(super) fund_unfinished_preset: bool,
    /// FEAT-0021 (AC-T8): the identity's names in every state of a contest.
    names_preset: bool,
    /// The lost name hidden (`contest-lost-hidden`): History keeps its lines (AC-T19).
    hidden_preset: bool,
    /// The contest card to open once the snapshot has the identity (`contest-card`, `contest-lost`).
    contest_preset: Option<&'static str>,
    /// FEAT-0048: the pool's states for «Create an identity» (`create-identity-pool-*`,
    /// `create-identity-core`), put into each snapshot.
    pool_identity_preset: Option<&'static str>,
    /// BUG-262 (`no-identity-*`): the wallet with no identity, in each snapshot; the Workshop's
    /// on the advanced level.
    no_identity_preset: Option<&'static str>,
    /// `--screen contact-accept` / `contact-resend` (BUG-249 r3): «Accept the request» over the
    /// demo's incoming request, «Send the requests again» over its first contact, once the
    /// snapshot has them.
    contact_preset: Option<&'static str>,
    /// `--screen fund-address-recovered`: the stopped one is a lock found on chain after a
    /// restore — no record of where it was signed to go, so it goes to the address shown now
    /// ([`recovered_to`]), and no InstantLock (BUG-171) — with «Continue» open.
    fund_recovered_preset: bool,
    /// `--screen tokens-*` (FEAT-0019): the search or the network as that frame of the design shows it.
    tokens_preset: Option<&'static str>,
    /// `--screen activity-fee-unknown` (BUG-176): the demo's profile saved on another device —
    /// its line found on the network, what it cost not known.
    fee_unknown_preset: bool,
    /// `--screen identity-keys-no-encryption`, `contacts-no-encryption` (FEAT-0020 K7): the demo
    /// identity without encryption keys, as one made in a wallet that gave it none — «Enable
    /// DashPay» on K1 and on Contacts.
    no_encryption_preset: bool,
    /// `--screen contacts-channel-broken` (BUG-231): @alice's request points to a disabled key —
    /// «Needs a new request» on her row, and a payment to her refused before the password.
    broken_channel_preset: bool,
    /// `--screen identity-keys-foreign`, `key-enable-dashpay-foreign` (FEAT-0020 AC-T19): the
    /// demo's key 6 another wallet's — «Encryption · unbound», and K7 over it.
    foreign_key_preset: bool,
    /// `--screen identity-keys-no-decryption`, `contacts-no-decryption` (BUG-286): the demo
    /// identity without its decryption key, as one made by another wallet — the missing keys'
    /// warning.
    no_decryption_preset: bool,
    /// `--screen identity-no-sign-in` (BUG-298): the demo's sign-in key 1 disabled.
    no_sign_in_preset: bool,
    /// `--screen overview-syncing` / `send-syncing`: the screen while the sync still runs.
    syncing_preset: bool,
    /// `--screen sync-stale-30`, `sync-stale-60` (SPV item 2a): synced, and no new block for 34
    /// or 125 minutes.
    stale_preset: Option<u32>,
    /// `--screen sync-platform` / `sync-platform-short` (BUG-137): Core synced, the identities and
    /// the tokens read from Platform; for 0.3 s only in the short one. When it began, and how long.
    platform_preset: Option<(std::time::Instant, Option<f64>)>,
    /// `--screen send-confirm-tall` / `edit-profile-password`: «Password for every send» on, so
    /// the dialog asks for it (BUG-122: the tallest form of each).
    password_preset: bool,
    /// `--screen send-unsignable` / `send-unsignable-short` (BUG-142): 0.001 DASH of Core no payment
    /// can sign for yet.
    unsignable_preset: bool,
    /// SPV Б2 and item 3: the coins an asset lock may take. `--screen top-up-many-coins` /
    /// `create-identity-many-coins`: 300 coins of 0.0001 DASH (`Some(true)`, more inputs than
    /// the network takes for 0.011); `top-up-not-final`: one final coin of 0.01 (`Some(false)`),
    /// the rest not confirmed yet.
    lock_coins_preset: Option<bool>,
    /// `--screen overview-mixed-unchecked` (FEAT-0030 O2): Core, and «Mixed» with it, last checked an
    /// hour ago.
    core_unchecked_preset: bool,
    /// `--screen welcome-other` (FEAT-0037 L2m): no wallets on this network, three on the other.
    empty_here_preset: bool,
    /// `--screen send-fiat-stale`: the rate older than 5 minutes (FEAT-0008, frame 3).
    stale_rate_preset: bool,
    /// `--screen send-pool-unread` / `send-pool-unknown`: Send from the pool before its first read
    /// in this run, with the amount kept from before (`true`) or none (BUG-113).
    pool_preset: Option<bool>,
    /// `--screen activity-notes-unreadable`: the notes file did not open (FEAT-0003, E2).
    pub(super) notes_error: Option<evolite_api::NotesError>,
    /// `--screen <part>-set-aside`: a file of the wallet set aside (BUG-204, BUG-212); gone from
    /// here once «Start over» went through.
    pub(super) set_aside: Vec<evolite_api::LocalFile>,
    /// Tests only: every Core card says that part of its details was not saved (FEAT-0005a, FR-6).
    pub(super) incomplete_details: bool,
    /// `--screen activity-card-long-title`: the demo's contact request is from a name long
    /// enough to reach the card's close cross (BUG-114).
    long_name: bool,
    /// `--screen activity-long-note`: History with an Arabic note too long for its row, cut with
    /// «…» at a word, its start kept (BUG-118).
    long_note: bool,
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
    /// FEAT-0004 B: «password for every send» on, so the spend asks it (the demo's is off).
    asks_password: bool,
}

impl PausePreset {
    fn of(dev: &DevLaunch) -> Option<Self> {
        let one = |tries| {
            Some(Self {
                one_wallet: true,
                wallet: None,
                tries,
                asks_password: false,
            })
        };
        let savings = Some(Self {
            one_wallet: false,
            wallet: Some(DEMO_OTHER),
            // The 8th wrong password: a pause of 5 min, 4:12 of it left (mockups 3 and 4).
            tries: Some((8, Some(252))),
            asks_password: false,
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
        } else if dev.preset("send-confirm-wait") || dev.preset("change-password-wait") {
            // FEAT-0004 B (AC-T3): the open wallet paused, the spend's or the change's form
            // over it — the 6th wrong password, 45 s left.
            Some(Self {
                one_wallet: false,
                wallet: None,
                tries: Some((6, Some(45))),
                asks_password: true,
            })
        } else {
            None
        }
    }
}

/// The `--screen` presets of the tokens screen that change the snapshot (design T1, T5, T4, T4b,
/// T3, T7).
const TOKEN_STATES: [&str; 16] = [
    "tokens-review",
    "tokens-send-credits",
    "tokens-first-search",
    "tokens-first-count",
    "tokens-balances",
    "tokens-again",
    "tokens-no-network-first",
    "tokens-pause",
    "tokens-no-network",
    "tokens-refresh-failed",
    "tokens-empty",
    "tokens-all-hidden",
    "tokens-mainnet",
    "tokens-claim-unknown",
    "tokens-page-destroy",
    "tokens-names",
];

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
            names_offline: dev.preset("names-offline"),
            preset_forgot: dev.preset("forgot"),
            remind_preset: dev.preset("remind"),
            unfinished_preset: dev.preset("identity-unfinished"),
            stuck_preset: dev.preset("activity-card-stuck"),
            core_preset: if dev.preset("create-identity-short") {
                Some(CorePreset::Short)
            } else if dev.preset("create-identity-unchecked") {
                Some(CorePreset::Unchecked)
            } else {
                None
            },
            second_identity_preset: dev.preset("identity-created"),
            long_identities_preset: LONG_IDENTITIES.into_iter().any(|p| dev.preset(p)),
            top_up_preset: if dev.preset("top-up-stopped") {
                Some(false)
            } else if dev.preset("top-up-awaiting") {
                Some(true)
            } else {
                None
            },
            contest_preset: if dev.preset("contest-card") {
                Some("nakamoto-dev")
            } else if dev.preset("contest-lost") {
                Some("bob")
            } else {
                None
            },
            contact_preset: ["contact-accept", "contact-resend"]
                .into_iter()
                .find(|name| dev.preset(name)),
            no_identity_preset: [
                "no-identity-contacts",
                "no-identity-tokens",
                "no-identity-collection",
                "no-identity-workshop",
                "no-identity-apps",
                "no-identity-identity",
            ]
            .into_iter()
            .find(|name| dev.preset(name)),
            pool_identity_preset: CORE_FRAMES
                .into_iter()
                .find(|name| dev.preset(name))
                .map(|_| "create-identity-core")
                .or_else(|| {
                    [
                        "create-identity-pool-short",
                        "create-identity-pool-connecting",
                        "create-identity-pool-off",
                        "create-identity-pool-checking",
                        "create-identity-pool-refused",
                        "create-identity-pool-refund",
                        "create-identity-pool-not-delivered",
                    ]
                    .into_iter()
                    .find(|name| dev.preset(name))
                }),
            hidden_preset: dev.preset("contest-lost-hidden"),
            names_preset: [
                "names-contested",
                "contest-card",
                "contest-lost",
                "contest-lost-hidden",
            ]
            .into_iter()
            .any(|p| dev.preset(p)),
            fund_unfinished_preset: dev.preset("fund-address-unfinished")
                || dev.preset("fund-address-resume")
                || dev.preset("fund-address-steps")
                || dev.preset("fund-address-recovered"),
            fund_recovered_preset: dev.preset("fund-address-recovered"),
            tokens_preset: TOKEN_STATES.into_iter().find(|name| dev.preset(name)),
            fee_unknown_preset: dev.preset("activity-fee-unknown"),
            no_encryption_preset: dev.preset("identity-keys-no-encryption")
                || dev.preset("contacts-no-encryption"),
            broken_channel_preset: dev.preset("contacts-channel-broken"),
            foreign_key_preset: dev.preset("identity-keys-foreign")
                || dev.preset("key-enable-dashpay-foreign"),
            no_decryption_preset: dev.preset("identity-keys-no-decryption")
                || dev.preset("contacts-no-decryption"),
            no_sign_in_preset: dev.preset("identity-no-sign-in"),
            syncing_preset: dev.preset("overview-syncing") || dev.preset("send-syncing"),
            stale_preset: if dev.preset("sync-stale-30") {
                Some(34)
            } else if dev.preset("sync-stale-60") {
                Some(125)
            } else {
                None
            },
            platform_preset: if dev.preset("sync-platform") {
                Some((std::time::Instant::now(), None))
            } else if dev.preset("sync-platform-short") {
                Some((std::time::Instant::now(), Some(0.3)))
            } else {
                None
            },
            password_preset: dev.preset("send-confirm-tall")
                || dev.preset("edit-profile-password")
                || dev.preset("send-mixed-agree")
                || dev.preset("send-mixed-changed")
                || dev.preset("fund-address-review")
                || dev.preset("fund-address-resume"),
            core_unchecked_preset: dev.preset("overview-mixed-unchecked"),
            empty_here_preset: dev.preset("welcome-other"),
            unsignable_preset: dev.preset("send-unsignable") || dev.preset("send-unsignable-short"),
            lock_coins_preset: if dev.preset("top-up-many-coins")
                || dev.preset("create-identity-many-coins")
            {
                Some(true)
            } else if dev.preset("top-up-not-final") {
                Some(false)
            } else {
                None
            },
            stale_rate_preset: dev.preset("send-fiat-stale"),
            pool_preset: if dev.preset("send-pool-unread") {
                Some(true)
            } else if dev.preset("send-pool-unknown") {
                Some(false)
            } else {
                None
            },
            incomplete_details: false,
            long_name: dev.preset("activity-card-long-title"),
            long_note: dev.preset("activity-long-note"),
            notes_error: dev
                .preset("activity-notes-unreadable")
                .then_some(evolite_api::NotesError::PutAside),
            set_aside: {
                use evolite_api::LocalFile::*;
                [
                    ("history-set-aside", Journal),
                    ("overview-set-aside", Journal),
                    ("top-ups-set-aside", TopUps),
                    ("names-set-aside", NameRequests),
                    ("apps-set-aside", ConnectedApps),
                    ("contacts-set-aside", ContactAddresses),
                    ("address-book-set-aside", AddressBook),
                    ("backup-set-aside", AutoBackup),
                ]
                .into_iter()
                .filter(|(name, _)| dev.preset(name))
                .map(|(_, file)| file)
                .collect()
            },
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
    /// FEAT-0004 B (tests): the pause of a `*-wait` preset over — no wrong passwords, no end —
    /// in the snapshot shown and in the ones to come; the rest of the preset stays.
    #[cfg(test)]
    pub(super) fn end_pause_preset(&mut self) {
        if let Some(pp) = self.dev.pause_preset.as_mut() {
            pp.tries = Some((0, None));
        }
        if let Some(s) = self.snapshot.as_mut() {
            for w in &mut s.wallets {
                w.wrong_tries = 0;
                w.wait_until = None;
            }
        }
    }

    pub(super) fn dev_snapshot(&mut self, ctx: &egui::Context) {
        if let Some(which) = self.dev.no_identity_preset
            && let Some(s) = self.snapshot.as_mut()
        {
            if which == "no-identity-workshop" {
                s.advanced_features = true;
            }
            if let Some(w) = s.wallet.as_mut() {
                w.identity = None;
                w.identities.clear();
                w.unfinished_identity = None;
                w.identity_search = evolite_api::IdentitySearch::Done;
            }
        }
        if let Some(which) = self.dev.pool_identity_preset
            && let Some(s) = self.snapshot.as_mut()
        {
            pool_identity_state(s, which);
        }
        if self.dev.names_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
            && let Some(id) = w.identity.as_mut()
        {
            let now = chrono::Local::now().timestamp();
            id.names = contested_names(&id.id, now);
            if self.dev.hidden_preset
                && let Some(c) = id
                    .names
                    .iter_mut()
                    .find(|n| n.label == "bob")
                    .and_then(|n| n.contest.as_mut())
            {
                c.listed = false;
            }
            // History's lines of the claims and of the lost one's result (N6).
            let who = evolite_api::IdentityRef {
                id: id.id.clone(),
                name: id.name().map(str::to_owned),
            };
            if !w.activity.iter().any(|a| a.id == CLAIM_IDS) {
                w.activity.splice(
                    0..0,
                    contest_lines(&who, now, w.fees.name_contested, w.fees.name_contest_fund),
                );
            }
        }
        if let Some(label) = self.dev.contest_preset
            && let Some(id) = self
                .snapshot
                .as_ref()
                .and_then(|s| s.wallet.as_ref())
                .and_then(|w| w.identity.as_ref())
        {
            self.dev.contest_preset = None;
            self.identity.open_contest(&id.id.clone(), label);
        }
        if let Some(which) = self.dev.contact_preset
            && let Some(w) = self.snapshot.as_ref().and_then(|s| s.wallet.as_ref())
        {
            use evolite_api::ContactState;
            let first = |state: ContactState| {
                w.contacts
                    .iter()
                    .find(|c| c.state == state)
                    .map(|c| c.identity_id.clone())
            };
            let opened = if which == "contact-accept" {
                first(ContactState::Incoming).map(|id| self.contacts.accepting = Some(id))
            } else {
                first(ContactState::Established).map(|id| {
                    self.contacts.resend =
                        Some(screens::identity::resend::ResendWindow::new(Some(id)));
                })
            };
            if opened.is_some() {
                self.dev.contact_preset = None;
            }
        }
        if self.dev.empty_here_preset
            && let Some(s) = self.snapshot.as_mut()
        {
            s.wallets.clear();
        }
        if self.dev.stale_rate_preset
            && let Some(rate) = self.snapshot.as_mut().and_then(|s| s.fiat.as_mut())
        {
            rate.at = chrono::Local::now().timestamp() - 2 * 60 * 60;
        }
        if self.dev.unfinished_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            w.unfinished_identity = Some(evolite_api::UnfinishedIdentity {
                identity: None,
                locked: Some(31_000_000),
                name: None,
            });
        }
        if self.dev.stuck_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
            && let Some(line) = w.activity.iter_mut().find(|a| a.id == 3)
            && let Some(detail) = line.detail.as_mut()
        {
            detail.resendable = true;
            if detail.resent_at.is_none() {
                line.proof = evolite_api::Proof::Unconfirmed;
                line.block = None;
            }
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
        if self.dev.long_identities_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
            && let Some(first) = w.identities.first()
            && w.identities.len() != 10
        {
            w.identities = long_identities(first);
            // FEAT-0055: the NFTs spread over them, so the Collection's filter has something
            // in each identity's.
            for (k, nft) in w.nfts.iter_mut().enumerate() {
                let owner = &w.identities[k % w.identities.len()];
                nft.owner_id = owner.id.clone();
                nft.owner = crate::components::identity_full_name(owner);
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
        if self.dev.fund_unfinished_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            let recovered = self.dev.fund_recovered_preset;
            w.unfinished_address_top_up = Some(evolite_api::UnfinishedAddressTopUp {
                locked: 10_012_000,
                // After a restore: the address shown now, not the one signed for (BUG-171).
                address: if recovered {
                    recovered_to()
                } else {
                    w.platform_address.clone()
                },
                on_top: w.fees.address_fund_on_top(),
                holds_back: true,
            });
        }
        if let Some(name) = self.dev.tokens_preset
            && let Some(s) = self.snapshot.as_mut()
        {
            tokens_state(s, name);
        }
        if self.dev.foreign_key_preset
            && let Some(key) = self
                .snapshot
                .as_mut()
                .and_then(|s| s.wallet.as_mut())
                .and_then(|w| w.identity.as_mut())
                .and_then(|i| i.keys.iter_mut().find(|k| k.id == 6))
        {
            key.owner = evolite_api::KeyOwner::Other;
        }
        // The decryption key goes too where it would be DashPay's instead of key 6 (AC-T19's
        // frames are of an identity without one); and in BUG-286's.
        if (self.dev.foreign_key_preset || self.dev.no_decryption_preset)
            && let Some(id) = self
                .snapshot
                .as_mut()
                .and_then(|s| s.wallet.as_mut())
                .and_then(|w| w.identity.as_mut())
        {
            id.keys.retain(|k| !k.decrypts);
        }
        if self.dev.no_sign_in_preset
            && let Some(key) = self
                .snapshot
                .as_mut()
                .and_then(|s| s.wallet.as_mut())
                .and_then(|w| w.identity.as_mut())
                .and_then(|i| i.keys.iter_mut().find(|k| k.id == 1))
        {
            key.disabled_at = Some(1_762_300_800);
        }
        if self.dev.no_encryption_preset
            && let Some(id) = self
                .snapshot
                .as_mut()
                .and_then(|s| s.wallet.as_mut())
                .and_then(|w| w.identity.as_mut())
        {
            id.keys
                .retain(|k| k.purpose != evolite_api::KeyPurpose::Encryption);
        }
        if self.dev.broken_channel_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            for c in w
                .contacts
                .iter_mut()
                .filter(|c| c.name.as_deref() == Some("alice"))
            {
                c.channel_broken = true;
            }
        }
        if self.dev.fee_unknown_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            for act in w
                .activity
                .iter_mut()
                .filter(|a| a.kind == evolite_api::ActivityKind::ProfileSaved)
            {
                act.amount = 0;
                act.detail.get_or_insert_default().cost_unknown = true;
            }
        }
        if self.dev.password_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            w.password_each_send = true;
        }
        // `--screen overview-syncing` / `send-syncing`: in the middle of the block filters, as a
        // restored wallet on testnet is for minutes.
        if self.dev.unsignable_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            w.fees.core_unsignable = 100_000;
        }
        if let Some(many) = self.dev.lock_coins_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            let values = if many {
                vec![10_000; 300]
            } else {
                vec![1_000_000]
            };
            let coins: Vec<_> = values
                .into_iter()
                .enumerate()
                .map(|(n, value)| evolite_api::CoreCoin {
                    value,
                    fee: 226 + 148 * n as evolite_api::Duffs,
                })
                .collect();
            if many {
                w.fees.core_coins = coins.clone();
            }
            w.fees.lock_coins = coins;
            w.fees.lock_inputs_max = 100;
        }
        if self.dev.core_unchecked_preset
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            w.balances.core_as_of = Some(evolite_api::CoreAsOf::Checked(
                chrono::Utc::now().timestamp() - 60 * 60,
            ));
        }
        if let Some(minutes) = self.dev.stale_preset
            && let Some(s) = self.snapshot.as_mut()
        {
            s.sync.stage = evolite_api::SyncStage::Done;
            s.sync.stale_minutes = Some(minutes);
        }
        if self.dev.syncing_preset
            && let Some(s) = self.snapshot.as_mut()
        {
            s.sync.stage = evolite_api::SyncStage::Filters;
            s.sync.progress = Some(0.58);
            s.sync.chainlocked = false;
        }
        if let Some((began, lasts)) = self.dev.platform_preset
            && let Some(s) = self.snapshot.as_mut()
            && lasts.is_none_or(|secs| began.elapsed().as_secs_f64() < secs)
        {
            s.sync.platform = vec![
                evolite_api::PlatformWork::Identities,
                evolite_api::PlatformWork::Tokens,
            ];
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
        // BUG-204: the files the preset set aside, where the real core would have put them.
        if !self.dev.set_aside.is_empty()
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            w.set_aside = self
                .dev
                .set_aside
                .iter()
                .map(|&file| evolite_api::SetAside {
                    file,
                    // BUG-256: a path of this system — a Windows one on macOS or Linux is one
                    // name, folder and all, and the banner would show the folder.
                    path: demo_set_aside_dir()
                        .join(format!("{file:?}.json.unreadable-1763302200"))
                        .to_string_lossy()
                        .into_owned(),
                })
                .collect();
        }
        if self.dev.incomplete_details
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
        {
            for d in w.activity.iter_mut().filter_map(|a| a.detail.as_mut()) {
                d.incomplete = true;
            }
        }
        if self.dev.long_note
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
            && let Some(line) = w.activity.iter_mut().find(|a| a.note.is_some())
        {
            line.note = Some(
                "مقابل تصميم الشعار والهوية البصرية الكاملة للمتجر الجديد في وسط المدينة · الفاتورة رقم 14"
                    .into(),
            );
        }
        if self.dev.long_name
            && let Some(w) = self.snapshot.as_mut().and_then(|s| s.wallet.as_mut())
            && let Some(line) = w.activity.iter_mut().find(|a| a.id == 14)
        {
            line.kind = evolite_api::ActivityKind::ContactRequest {
                to: "@qa-kestrel-1108b-with-a-name-long-enough-to-reach-the-cross".into(),
                accepting: true,
                ours: None,
            };
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
            if pp.asks_password
                && let Some(w) = s.wallet.as_mut()
            {
                w.password_each_send = true;
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
        // FEAT-0023 round 4: the request at its end (A1b), and the list's dialogs (R1, D1, D2)
        // or its disconnected part (L2).
        self.requests.body_to_end = preset("connect-limits") || preset("connect-renew");
        self.requests.preset = [
            ("apps-renew", "renew"),
            ("apps-disconnect", "disconnect"),
            ("apps-remove", "remove"),
            ("apps-disconnected", "to-end"),
        ]
        .into_iter()
        .find(|(name, _)| preset(name))
        .map(|(_, what)| what);
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
        // BUG-145: History on «Apps» and «Identity»: sign-ins, disconnects, what apps spent and
        // tokens sent.
        if preset("activity-apps") {
            self.history.filters.kinds =
                screens::activity_filters::kind::APPS | screens::activity_filters::kind::IDENTITY;
        }
        // FEAT-0030 H1: History on «Mixed».
        // FEAT-0019 T-1: History on «Tokens» (the designer's H1).
        if preset("activity-tokens") {
            self.history.filters.layer = screens::activity_filters::TOKENS_LAYER;
        }
        if preset("activity-mixed") {
            self.history.filters.layer = 2;
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
        // FEAT-0008: a card with the rate kept (frame 7) and one without (frame 8).
        if preset("activity-card-fiat") && !real_shot {
            screens::open_card(ctx, 2);
        }
        if preset("activity-card-fiat-old") && !real_shot {
            screens::open_card(ctx, 3);
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
            ("activity-card-stuck", 3),
            ("activity-card-pool-sent", 21),
            ("activity-card-pool-received", 22),
            ("activity-card-top-up", 4),
            ("activity-card-long-title", 14),
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
            // FEAT-0012: the tax category switched to a transfer on a payment out (frames 02,
            // 04) and in (frame 03), its window open, and a change the core refused.
            for (name, line) in [
                ("activity-card-tax-transfer", 3),
                ("activity-card-tax-explainer", 3),
                ("activity-card-tax-received", 8),
            ] {
                if preset(name) {
                    screens::open_card(ctx, line);
                    self.core.send(Command::SetTaxTransfer {
                        key: format!("demo:{line}"),
                        transfer: true,
                    });
                }
            }
            if preset("activity-card-tax-explainer") {
                screens::activity_tax::open_explainer(ctx);
            }
            if preset("activity-card-tax-error") {
                screens::open_card(ctx, 3);
                screens::activity_tax::tax_failed_example(ctx, "demo:3", true);
            }
            // A word of the demo's note.
            if preset("activity-search-note") {
                self.history.query =
                    crate::i18n::t_in(crate::i18n::current(), tr!("logo")).into_owned();
            }
        }
        // `--screen send-confirm`: the last look asks where the payment goes, as a click does.
        if preset("send-confirm")
            || preset("send-confirm-note")
            || preset("send-confirm-link")
            || preset("send-confirm-wait")
        {
            self.core.send(Command::PreviewPayment {
                to: "@alice".into(),
            });
        }
        if demo {
            // `--screen send-confirm` opens the last look before signing.
            self.send.confirming = preset("send-confirm")
                || preset("send-confirm-note")
                || preset("send-confirm-link")
                || preset("send-confirm-wait");
            // `--screen send-done` (BUG-169 S4): just after a payment — the form cleared, the
            // network's answer on it, «Done» in place of «Check and send».
            if preset("send-done") {
                self.send.recipient.clear();
                self.send.amount.clear();
                self.send.result = Some(Ok(evolite_api::SentPayment {
                    txid: "4f1c9e2b7a5d3086c1e4f7a2b9d0c3e6f8a1b4d7c0e3f6a9b2c5d8e1f4a7b0c3".into(),
                    amount: 25_000_000,
                    fee: 227,
                    relayed: true,
                }));
            }
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
            } else if preset("send-book") || preset("names-offline") {
                self.send.recipient.clear();
            }
            // FEAT-0015: paying from the Platform addresses: a transfer, a withdrawal costing much of
            // its amount, one to a foreign Core address, a top-up of the identity, the last look.
            let theirs = on_this_network("qqmz8fh2wl5c9u4hyrq3m4n0t7k2vx6s4u6aznp");
            let from_address = [
                ("send-from-address", theirs.as_str(), 5_000_000),
                ("address-confirm", theirs.as_str(), 5_000_000),
                (
                    "address-withdraw-warning",
                    "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f2c",
                    2_000_000,
                ),
                (
                    "address-withdraw-foreign",
                    "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ",
                    2_000_000,
                ),
                ("address-top-up", "@satoshi", 1_000_000),
                ("address-top-up-foreign", "@alice", 1_000_000),
            ];
            if let Some((name, to, amount)) = from_address.into_iter().find(|(n, ..)| preset(n)) {
                self.send.replace(screens::send::SendForm::paying(
                    evolite_api::Source::PlatformAddress,
                    to.into(),
                ));
                self.send.amount = crate::format::input_text(amount);
                self.send.suggested = Some(amount);
                self.send.confirming = name == "address-confirm";
            }
            // Core to someone else's Platform address (frame 15): the way there from elsewhere.
            if preset("core-to-platform-address") {
                self.send.recipient = on_this_network("qqmz8fh2wl5c9u4hyrq3m4n0t7k2vx6s4u6aznp");
            }
            // FEAT-0015 stage 2 (frame 14): Core to the wallet's own Platform address — a top-up of it.
            if preset("send-fund-own") {
                self.send.recipient = on_this_network("qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq");
                self.send.amount = crate::format::input_text(10_000_000);
            }
            // `--screen send-pool-unread` / `send-pool-unknown`: from the pool, to an address: the card,
            // the line under the row, «Left» and the button (BUG-113).
            // FEAT-0013: «Coins» on Send (F1–F3, P1) and its dialog (CC1–CC4), over the demo's
            // coins (`evolite_demo::demo::demo_coins`: n-th coin `<n repeated>:<n % 3>`).
            let coin = |n: u8| format!("{}:{}", format!("{n:02x}").repeat(32), n % 3);
            let coins_preset = [
                ("send-coins", &[][..], 0, false),
                ("send-coins-chosen", &[5u8, 6, 7][..], 12_000_000, false),
                ("send-coins-name", &[][..], 5_000_000, false),
                ("send-coins-confirm", &[5u8, 6, 7][..], 12_000_000, false),
                // BUG-122: the tallest last look — coins, the password, a look-alike address.
                ("send-confirm-tall", &[5u8, 6, 7][..], 12_000_000, false),
                ("coins-manual", &[4u8, 5][..], 5_000_000, true),
                ("coins-auto", &[][..], 5_000_000, true),
                ("coins-not-enough", &[6u8, 7][..], 20_000_000, true),
                ("coins-spent", &[6u8, 9][..], 5_000_000, true),
                // FEAT-0030 CC-M1, CC-M2: a coin not fully mixed alone, and with an ordinary one.
                ("coins-mixed", &[10u8][..], 500_000, true),
                ("coins-mixed-refused", &[4u8, 10][..], 500_000, true),
            ];
            if let Some((name, chosen, amount, dialog)) =
                coins_preset.into_iter().find(|(n, ..)| preset(n))
            {
                self.send.recipient = if name == "send-coins-name" {
                    "@alice".into()
                } else if name == "send-confirm-tall" {
                    self.core.send(Command::PreviewPayment {
                        to: LOOKALIKE.into(),
                    });
                    LOOKALIKE.into()
                } else {
                    "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f".into()
                };
                // As a suggestion, so the field writes it in the screen's number format (the
                // preset runs before the language's marks are set).
                self.send.suggested = (amount > 0).then_some(amount);
                self.send.amount = if amount > 0 {
                    crate::format::input_text(amount)
                } else {
                    String::new()
                };
                let set: std::collections::BTreeSet<String> =
                    chosen.iter().map(|n| coin(*n)).collect();
                if dialog {
                    self.send.coins.dialog = true;
                    self.send.coins.manual = name != "coins-auto";
                    self.send.coins.reveal_group = true;
                    self.send.coins.ticked = set;
                } else {
                    self.send.coins.chosen = set;
                }
                self.send.confirming = matches!(name, "send-coins-confirm" | "send-confirm-tall");
                self.core.send(self.send.coins.ask_again());
            }
            // FEAT-0035 (ADR2): the last look of a payment to an address only like the
            // address book's «Workshop rent» — characters 9–11 differ.
            if preset("send-lookalike") {
                self.send.recipient = LOOKALIKE.into();
                // As a suggestion, so the field writes it in the screen's number format.
                self.send.amount = crate::format::input_text(1_000_000);
                self.send.suggested = Some(1_000_000);
                self.send.confirming = true;
                self.core.send(Command::PreviewPayment {
                    to: LOOKALIKE.into(),
                });
            }
            // FEAT-0008: 25 USD typed (frame 1), its last look (frame 4), an old rate (frame 3).
            if preset("send-fiat") || preset("send-fiat-confirm") || preset("send-fiat-stale") {
                self.send.recipient = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f".into();
                self.send.in_currency = true;
                self.send.fiat = "25".into();
                self.send.fiat_typed = true;
                self.send.amount.clear();
                self.send.suggested = None;
                self.send.confirming = preset("send-fiat-confirm");
                if preset("send-fiat-stale") {
                    self.send.amount = crate::format::input_text(25_000_000);
                }
            }
            if preset("receive-fiat") {
                self.receive.in_currency = true;
                self.receive.fiat = "25".into();
            }
            // FEAT-0030 S1: from «Mixed» to an address; the demo's two 0.100001 coins pay it with an
            // excess under 0.001 DASH and 1 %, shown in the summary.
            // BUG-142: from Core with a part not to sign for; `-short` asks for the whole balance less
            // the fee, more than can be spent.
            if preset("send-unsignable") || preset("send-unsignable-short") {
                self.send.pick(evolite_api::Source::Core);
                self.send.recipient = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f".into();
                let amount = if preset("send-unsignable-short") {
                    841_199_000
                } else {
                    10_000_000
                };
                self.send.suggested = Some(amount);
                self.send.amount = crate::format::input_text(amount);
            }
            if preset("send-mixed") {
                self.send.pick(evolite_api::Source::Mixed);
                self.send.recipient = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f".into();
                self.send.suggested = Some(19_990_000);
                self.send.amount = crate::format::input_text(19_990_000);
            }
            // The rest of FEAT-0030's frames: S2 to a name, S3 an excess over 0.01 DASH, and the
            // last look — P1 a small excess, P2 one to agree to (the password asked), P2b the core
            // found another, P4 the core refused.
            let mixed_presets = [
                ("send-mixed-name", "@alice", 19_990_000, false),
                ("send-mixed-refused", "", 15_000_000, false),
                ("send-mixed-confirm", "", 19_990_000, true),
                ("send-mixed-agree", "", 19_500_000, true),
                ("send-mixed-changed", "", 19_500_000, true),
                ("send-mixed-refused-confirm", "", 15_000_000, true),
            ];
            if let Some((name, to, amount, confirming)) =
                mixed_presets.into_iter().find(|(n, ..)| preset(n))
            {
                self.send.pick(evolite_api::Source::Mixed);
                self.send.recipient = if to.is_empty() {
                    "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f".into()
                } else {
                    to.into()
                };
                self.send.suggested = Some(amount);
                self.send.amount = crate::format::input_text(amount);
                self.send.confirming = confirming;
                if name == "send-mixed-changed" {
                    // As after the core's answer to a payment with another excess agreed.
                    self.send.mixed.changed = Some(499_860);
                }
            }
            if preset("send-pool-unread") || preset("send-pool-unknown") {
                self.send.pick(evolite_api::Source::Pool);
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
        // `--screen edit-profile` opens 23 over Identity; `edit-profile-imgur-page` with a link
        // to an Imgur album in it (FEAT-0018, frame AV2).
        let imgur_page = preset("edit-profile-imgur-page");
        if (preset("edit-profile") || preset("edit-profile-password") || imgur_page) && !real_shot {
            self.identity.editing = Some(screens::identity::ProfileForm {
                display: "Satoshi Nakamoto".into(),
                about: tr!("I write about Dash Platform and collect generative NFTs. Pay through a DashPay contact — that way each payment goes to a new address.").into(),
                avatar_url: if imgur_page {
                    "https://imgur.com/a/Xq3b9Tz"
                } else {
                    "https://ipfs.io/ipfs/bafybeigdyrz/satoshi.png"
                }
                .into(),
                ..Default::default()
            });
        }
        // `--screen top-up` opens "Top up the identity" over Identity.
        // `--screen top-up-bad-amount`: with «1,234.56» typed, the line under the field (BUG-161).
        // `--screen top-up-many-coins` / `top-up-not-final` (SPV Б2, item 3): 0.011 and 0.1 DASH
        // the coins cannot lock.
        let lock_coins = preset("top-up-many-coins") || preset("top-up-not-final");
        // FEAT-0048 UC-7: `top-up-pool` opens by the order (the pool first), `top-up-mixed` from
        // fully mixed coins with 0.05 typed and the coins' plan put in (the demo answers «not
        // enough»); the frames before them are Core's.
        let private = preset("top-up-pool") || preset("top-up-mixed");
        self.identity.top_up =
            (preset("top-up") || preset("top-up-bad-amount") || lock_coins || private).then(|| {
                let mut form = screens::identity::TopUpForm::default();
                if !private {
                    form.from = Some(screens::identity::TopUpFrom::Core);
                }
                if preset("top-up-mixed") {
                    form.from = Some(screens::identity::TopUpFrom::Mixed);
                    form.amount = crate::format::input_text(5_000_000);
                    form.suggested = Some(5_000_000);
                    form.mixed.set(
                        evolite_api::MixedTo::TopUp,
                        5_000_000,
                        Ok(evolite_api::MixedLockQuote {
                            amount: 5_000_000,
                            nominal: 10_000_000,
                            exact: false,
                            excess: 5_000_000,
                            confirm: true,
                            coins: 1,
                            fee: 2_260,
                            exactly: Some(3_000_000),
                        }),
                    );
                } else if preset("top-up-bad-amount") {
                    form.amount = "1,234.56".into();
                } else if lock_coins {
                    let amount = if preset("top-up-many-coins") {
                        1_100_000
                    } else {
                        10_000_000
                    };
                    form.amount = crate::format::input_text(amount);
                    form.suggested = Some(amount);
                } else if !real_shot {
                    // The frame with an amount, as the person would type it; the form opens empty
                    // (BUG-182).
                    form.amount = crate::format::input_text(10_000_000);
                    form.suggested = Some(10_000_000);
                }
                form
            });
        // FEAT-0015 stage 2: the funding of the Platform address frame by frame (22–25).
        if preset("fund-address") || preset("fund-address-review") {
            // The frames with an amount (22, 22b): 0.1 DASH, put in as the person would type it.
            // The form itself opens empty (BUG-172, frame F1).
            self.fund_address.form = Some(screens::fund_address::FundForm {
                amount: crate::format::input_text(10_000_000),
                suggested: Some(10_000_000),
                review: preset("fund-address-review"),
            });
        }
        if preset("fund-address-steps") {
            self.fund_address
                .flow
                .set_stage(crate::components::spend_flow::Stage::Waiting {
                    ask: 0,
                    signed: 10_000_000,
                });
            self.fund_address.steps_open = true;
        }
        self.fund_address.resume_open =
            preset("fund-address-resume") || preset("fund-address-recovered");
        if preset("fund-address-done") {
            self.fund_address
                .flow
                .set_stage(crate::components::spend_flow::Stage::Answered {
                    signed: 10_000_000,
                    result: Ok(()),
                });
        }
        // `--screen add-name` opens "Add a name" with a name being checked.
        self.identity.adding_name = preset("add-name").then(|| screens::identity::NameForm {
            name: "satoshi-nakamoto-2026".into(),
            ..Default::default()
        });
        // FEAT-0021: «Add a name» on a contested name (N1), its review (N2), the fund risen
        // since (R-2), and the request sent (N3).
        if preset("add-name-contested") {
            self.identity.adding_name = Some(screens::identity::NameForm {
                name: "sam".into(),
                ..Default::default()
            });
        }
        if preset("add-name-review") || preset("add-name-sent") || preset("add-name-fund-rose") {
            let now = chrono::Local::now().timestamp();
            let fund = 10_000_000;
            self.identity.adding_name = Some(screens::identity::NameForm {
                name: "sam".into(),
                asked: "sam".into(),
                check: Some(Ok(evolite_api::NameCheck::NeedsVote)),
                result: preset("add-name-sent").then_some(Ok(())),
                review: Some(screens::identity::Review {
                    fund: if preset("add-name-fund-rose") {
                        2 * fund
                    } else {
                        fund
                    },
                    was: preset("add-name-fund-rose").then_some(fund),
                    join_until: now + 45 * 60,
                    ends_about: now + 90 * 60,
                }),
                ..Default::default()
            });
        }
        // FEAT-0048 UC-5: the advanced level's «Create from Core (not private)» chosen — and so
        // for the frames of the Core form (FEAT-0014), which keep showing it.
        self.create_identity.pool.core = CORE_FRAMES.into_iter().any(&preset);
        // FEAT-0048: «Create privately», its review open over the form.
        self.create_identity.pool.confirming = preset("create-identity-pool-review");
        // UC-4 «Shield again» open over the refusal plate.
        self.create_identity.pool.refund =
            preset("create-identity-pool-refund").then_some(2_742_313);
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
            // `--screen unshield-bad-amount`: «1,234.56» typed, the line under the field (BUG-161).
            let bad = preset("unshield-bad-amount");
            let unshield = preset("unshield") || preset("unshield-identity") || bad;
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
                unshield_to: if preset("unshield-identity") {
                    screens::shielded::UnshieldTo::Identity
                } else {
                    screens::shielded::UnshieldTo::Core
                },
                unshield_amount: if bad {
                    "1,234.56".into()
                } else if unshield && !real_shot {
                    crate::format::input_text(30_000_000)
                } else {
                    String::new()
                },
                unshield_suggested: (unshield && !real_shot).then_some(30_000_000),
                address_open: preset("pool-address"),
                ..Default::default()
            }
        };
        // `--screen tokens-send*` / `tokens-review` / `tokens-add*`: the tokens screen's dialogs
        // (design T6, T6b and its refusals; «Add a token» and its results), over the demo's Evo
        // Coffee. A name in «To» is asked of the demo core, as «Check» asks it.
        let send_preset = [
            ("tokens-send", "", ""),
            ("tokens-review", "@alice", "5"),
            (
                "tokens-send-address",
                "yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh",
                "5",
            ),
            ("tokens-send-more", "@alice", "1000"),
            ("tokens-send-credits", "@alice", "5"),
            ("tokens-send-unknown", "@nobody-here", "5"),
        ]
        .into_iter()
        .find(|(name, _, _)| preset(name));
        if let Some((name, to, amount)) = send_preset {
            let mut send = screens::tokens::SendState::new(DEMO_TOKEN);
            if !real_shot {
                send.to = to.into();
                send.amount = amount.into();
                if matches!(name, "tokens-review" | "tokens-send-unknown") {
                    send.checking = Some(to.into());
                    self.core.send(Command::ResolveName { name: to.into() });
                }
            }
            self.tokens.sending = Some(send);
        }
        // `--screen tokens-claim*` / `tokens-buy*` (FEAT-0019 T-2): K1, K1u, P1 and P2 over Evo
        // Coffee. P2: the purchase was refused for a price that grew from 0.0006 DASH.
        if preset("tokens-claim") || preset("tokens-claim-unknown") {
            self.tokens.claiming = Some(screens::tokens::ClaimState::new(DEMO_TOKEN));
        }
        if preset("tokens-buy") || preset("tokens-buy-changed") {
            let mut buy = screens::tokens::BuyState::new(DEMO_TOKEN);
            if !real_shot {
                buy.count = "15".into();
                if preset("tokens-buy-changed") {
                    buy.was = Some(600_000);
                }
            }
            self.tokens.buying = Some(buy);
        }
        // `--screen tokens-page` and the issuer windows over it (FEAT-0019 T-3: M1, M2, M3, M4,
        // M5, M6, M7) on the demo's Evo Coffee, whose page the demo core reads.
        let issuer_preset = [
            ("tokens-page", None),
            ("tokens-page-destroy", None),
            (
                "tokens-names",
                Some(screens::tokens::manage::IssuerWindow::Setting(
                    evolite_api::tokens::SettingField::Names,
                    screens::tokens::manage::SettingEdit::Value,
                )),
            ),
            (
                "tokens-mint",
                Some(screens::tokens::manage::IssuerWindow::Mint),
            ),
            (
                "tokens-burn",
                Some(screens::tokens::manage::IssuerWindow::Burn),
            ),
            (
                "tokens-destroy",
                Some(screens::tokens::manage::IssuerWindow::Destroy(
                    "9Xc4Wn2Pk7Lm3Qr8Ts5Vy1Bz6Hd4Fj2Gs9Ku3Mp7Nq".into(),
                )),
            ),
            (
                "tokens-suspend",
                Some(screens::tokens::manage::IssuerWindow::Pause),
            ),
            (
                "tokens-price",
                Some(screens::tokens::manage::IssuerWindow::Price),
            ),
            (
                "tokens-setting",
                Some(screens::tokens::manage::IssuerWindow::Setting(
                    evolite_api::tokens::SettingField::Who(evolite_api::tokens::IssuerAction::Mint),
                    screens::tokens::manage::SettingEdit::Makers,
                )),
            ),
        ]
        .into_iter()
        .find(|(name, _)| preset(name));
        if let Some((_, window)) = issuer_preset {
            self.tokens.page = Some(screens::tokens::manage::PageState::new(DEMO_TOKEN));
            if let Some(window) = window {
                let mut form = screens::tokens::manage::IssuerForm::new(DEMO_TOKEN, window);
                if !real_shot {
                    form.amount = "250".into();
                    form.price_kind = 2;
                    form.tiers = vec![
                        ("1".into(), "0.001".into()),
                        ("100".into(), "0.0008".into()),
                    ];
                }
                self.tokens.issuer = Some(form);
            }
        }
        let add_preset = [
            ("tokens-add", ""),
            ("tokens-add-one", DEMO_TOKEN),
            ("tokens-add-many", "coffee"),
            ("tokens-add-none", "nothing"),
            ("tokens-add-hidden", "coffee"),
        ]
        .into_iter()
        .find(|(name, _)| preset(name));
        // `tokens-add-hidden`: Evo Coffee hidden first, so the search offers «Return to the list».
        if preset("tokens-add-hidden") && !real_shot {
            self.core.send(Command::TrackToken {
                token_id: DEMO_TOKEN.into(),
                on: false,
            });
        }
        if let Some((_, query)) = add_preset {
            match screens::tokens::AddState::asking(query).filter(|_| !real_shot) {
                Some((add, command)) => {
                    self.core.send(command);
                    self.tokens.adding = Some(add);
                }
                None => self.tokens.adding = Some(screens::tokens::AddState::default()),
            }
        }
        // `--screen welcome-other` (FEAT-0037 L2m): the other network has three wallets.
        if preset("welcome-other") {
            self.networks.other_wallets = 3;
        }
        // `--screen network-switch` (FEAT-0037 N2d, N2t): Settings → Network with the switch's
        // question, to the other network.
        if preset("network-switch") {
            self.settings.switch_to = Some(super::network::other(evolite_core::network::current()));
        }
        // `--screen identity-key-shown` (FEAT-0020 K6b): K1 with key 2 shown — the demo's test key.
        if preset("identity-key-shown") {
            self.identity_keys.reveal_for_preset();
        }
        // `--screen key-disable` and its kin (FEAT-0020): K3, K4, K3b, K5 over K1.
        for name in [
            "key-disable",
            "key-replace-main",
            "key-replace-dashpay",
            "key-add",
            "key-enable-dashpay-foreign",
        ] {
            if preset(name) {
                self.identity_keys.change_for_preset(name);
            }
        }
        // `--screen gift` opens the collection with the gift dialog.
        self.collection.gifting = preset("gift");
        // `--screen phrase` opens 17 over Backup, `--screen appearance` that section.
        let report = preset("report") || preset("report-letter");
        self.settings.section = if preset("phrase") || preset("check-phrase") || preset("backup") {
            Section::Backup
        } else if preset("appearance") || preset("appearance-rate") || preset("language") {
            Section::Appearance
        } else if preset("change-password")
            || preset("change-password-wait")
            || preset("delete")
            || preset("delete-notes")
            || preset("security")
        {
            Section::Security
        } else if preset("about") || preset("about-end") || report {
            Section::About
        } else if preset("notifications") {
            Section::Notifications
        } else if preset("advanced") || preset("advanced-workshop") {
            Section::Advanced
        } else if preset("wallets") || preset("wallet-rename") || preset("wallet-more") {
            Section::Wallets
        } else {
            Section::Network
        };
        self.settings.phrase_dialog = preset("phrase");
        // FEAT-0038: `--screen advanced-workshop` W0 with «Show the Workshop» on; `workshop*` the
        // Workshop's pages (W1, W2, W3, the steps, W4) with it on.
        let workshop = [
            "advanced-workshop",
            "workshop",
            "no-identity-workshop",
            "workshop-identities-long",
            "workshop-new",
            "workshop-review",
            "workshop-steps",
            "workshop-collection",
            "workshop-model",
            "workshop-model-copy",
            "workshop-model-page",
            "workshop-give",
            "workshop-issue",
            "workshop-model-review",
        ];
        if workshop.into_iter().any(preset) {
            self.settings.set("workshop", true);
        }
        if preset("workshop-new") || preset("workshop-review") || preset("workshop-steps") {
            self.workshop.page = screens::workshop::Page::New;
            self.workshop.new.preset(
                "Dash Pins",
                preset("workshop-review"),
                preset("workshop-steps"),
            );
        }
        // W5 with a photo too large for its record (`-copy`: its smaller copy made).
        if preset("workshop-model") || preset("workshop-model-copy") {
            let contract = "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P";
            self.workshop.model = screens::workshop::ModelForm::new(contract);
            self.workshop.page = screens::workshop::Page::NewModel(contract.into());
            let photo = image::RgbaImage::from_fn(240, 160, |x, y| {
                let n = ((x * 7919 + y * 104_729) % 97) as u8;
                image::Rgba([(x as u8) ^ n, (y as u8).wrapping_mul(3), n, 255])
            });
            let mut png = std::io::Cursor::new(Vec::new());
            let _ = photo.write_to(&mut png, image::ImageFormat::Png);
            let fit = self.workshop.model.preset(
                "Orange pin",
                "orange-pin.png",
                png.into_inner(),
                preset("workshop-model-copy"),
            );
            if let Some(command) = fit {
                self.core.send(command);
            }
        }
        // W7: the demo's example model; W8 «Give» or «Issue more» open over it.
        if preset("workshop-model-page") || preset("workshop-give") || preset("workshop-issue") {
            self.workshop.page = screens::workshop::Page::Model {
                contract: "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P".into(),
                model: "7Yowk46VwwHqmD5yZyyygggh937aP6h2UW7aQWBdWpM5".into(),
            };
            let action = if preset("workshop-give") {
                Some(screens::workshop::SeriesAction::Give)
            } else if preset("workshop-issue") {
                Some(screens::workshop::SeriesAction::Issue)
            } else {
                None
            };
            let identity = self
                .snapshot
                .as_ref()
                .and_then(|s| s.wallet.as_ref())
                .and_then(|w| w.identity.as_ref())
                .map(|i| i.id.clone())
                .unwrap_or_default();
            self.workshop.series = action.map(|action| {
                screens::workshop::SeriesForm::preset(
                    action,
                    evolite_api::series::SeriesTarget {
                        identity,
                        contract: "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P".into(),
                        model: "7Yowk46VwwHqmD5yZyyygggh937aP6h2UW7aQWBdWpM5".into(),
                    },
                )
            });
        }
        // W5's «Check the model» open (BUG-249: its summary in the sweeps).
        if preset("workshop-model-review") {
            let contract = "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P";
            self.workshop.model = screens::workshop::ModelForm::new(contract);
            self.workshop.page = screens::workshop::Page::NewModel(contract.into());
            self.workshop.model.preset_review("Orange pin");
        }
        if preset("workshop-collection") {
            // The demo core's example collection (`evolite_demo::workshop::CONTRACT`).
            self.workshop.page = screens::workshop::Page::Collection(
                "FzKL9288ZzoQcXKWvXc6kRwbNciboLpJnX4egZo9iz9P".into(),
            );
        }
        // `--screen create-identity` shows a name, as in Figma; `--screen creating` the
        // same while the core is on step 2.
        if preset("create-identity") || preset("creating") || preset("create-identity-confirm") {
            self.create_identity.name = "evolite-2026".into();
        }
        // `--screen create-identity-many-coins` (SPV Б2): a nameless identity of 0.011 DASH from
        // 300 coins of 0.0001.
        if preset("create-identity-many-coins") {
            self.create_identity.amount = crate::format::input_text(1_100_000);
        }
        // `--screen create-identity-confirm`: «Check the new identity» over it (FEAT-0014,
        // frame 7); `create-identity-nameless`, `-short`, `-unchecked` keep the name empty.
        self.create_identity.confirming = preset("create-identity-confirm");
        // `--screen identity-created`: the banner after a creation (frame 5).
        self.identity.created =
            preset("identity-created").then_some(screens::identity::Created::Awaiting(None));
        self.create_identity.set_waiting(preset("creating"));
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
        if preset("change-password") || preset("change-password-wait") {
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
        if self.dev.pause_preset.is_some()
            && !preset("wallet-switch-wait")
            && !preset("send-confirm-wait")
            && !preset("change-password-wait")
        {
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
        // FEAT-0008: «Fiat rate» on, USD, for the amounts in the currency.
        let fiat = [
            "send-fiat",
            "send-fiat-confirm",
            "send-fiat-stale",
            "receive-fiat",
            "activity-fiat",
            "activity-card-fiat",
            "activity-card-fiat-old",
        ];
        if fiat.iter().any(|p| dev.preset(p)) {
            self.settings.set("fiat-rate", true);
            self.settings.fiat_currency = 0;
        }
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

/// `--screen send-lookalike`: the demo address book's «Workshop rent»
/// (`yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh`) with characters 9–11 changed (FEAT-0035).
pub(crate) const LOOKALIKE: &str = "yNk2Hc8pQQQmR7vT3sLb9XeJ6uZa5Df1Gh";

/// The demo's Evo Coffee: the token the send presets open (`evolite-demo`, `tokens.rs`).
const DEMO_TOKEN: &str = "7Hq2Wd9kLm4Np8Rs3Tv6Xy1Zb5Cf2Gh7Jk9Mn4Pq8Rs";

/// The identity's names in every state of a contest (FEAT-0021, AC-T8, `--screen
/// names-contested`): registered, in the vote, waiting for the result, won, lost and blocked.
pub(crate) fn contested_names(identity: &str, now: i64) -> Vec<evolite_api::DpnsName> {
    use evolite_api::{DpnsName, NameContest, NameState};
    let day = 86_400;
    let other = "9vXqKpTzR4mWdEh8LcYb2NsFgJuA6oQi3t7nZk5yHwC";
    let contest = |ours: u32, theirs: u32, winner: Option<&str>, listed| NameContest {
        contenders: vec![(identity.to_owned(), ours), (other.to_owned(), theirs)],
        winner: winner.map(str::to_owned),
        fund: 10_000_000,
        sent_at: now - 12 * day,
        checked_at: Some(now - 90),
        listed,
        notice_closed: false,
        result_notice_closed: false,
        link: None,
        link_written: false,
    };
    let name = |label: &str, state, contest| DpnsName {
        label: label.into(),
        primary: false,
        state,
        contest,
    };
    let mut names = vec![
        name("satoshi", NameState::Registered, None),
        name(
            "nakamoto-dev",
            NameState::Voting {
                join_until: now + 2 * day,
                ends_about: now + 9 * day,
                contenders: 3,
                ours: 12,
                lock: 3,
                abstain: 1,
            },
            // АРХ-6: a link the claim kept that Platform did not take (AC-T12).
            Some(NameContest {
                link: Some("https://example.com/sam".into()),
                ..contest(12, 9, None, true)
            }),
        ),
        name(
            "sam",
            NameState::AwaitingResult {
                ends_about: now - 3600,
            },
            Some(contest(7, 7, None, true)),
        ),
        name(
            "won",
            NameState::Won { at: now - 5 * day },
            Some(contest(20, 4, Some(identity), true)),
        ),
        name(
            "bob",
            NameState::Lost { at: now - 24 * day },
            Some(contest(3, 14, Some(other), true)),
        ),
        name(
            "zed",
            NameState::Locked { at: now - 2 * day },
            Some(contest(2, 1, None, true)),
        ),
    ];
    names[0].primary = true;
    names
}

/// FEAT-0048: the frames of the Core form (FEAT-0014, the designer's 4–10): on the advanced
/// level, «Create from Core (not private)» chosen, as the person gets there now (UC-5).
/// `create-identity-pool` is the default, from the pool.
const CORE_FRAMES: [&str; 8] = [
    "create-identity",
    "create-identity-nameless",
    "create-identity-confirm",
    "create-identity-short",
    "create-identity-unchecked",
    "create-identity-many-coins",
    "creating",
    "create-identity-core",
];

/// FEAT-0048: the pool as each `create-identity-pool-*` frame shows it — too little in it, its
/// first check, unavailable (UC-6), sent and not known yet (UC-3), refused (UC-4) — and the
/// advanced level for `create-identity-core` (UC-5).
fn pool_identity_state(s: &mut evolite_api::Snapshot, which: &str) {
    if which == "create-identity-core" {
        s.advanced_features = true;
    }
    let Some(w) = s.wallet.as_mut() else {
        return;
    };
    let now = chrono::Local::now().timestamp();
    match which {
        "create-identity-pool-short" => w.balances.shielded = 1_200_000,
        "create-identity-pool-connecting" => {
            w.pool.connecting = true;
            w.pool.checked_at = None;
        }
        "create-identity-pool-off" => {
            w.pool.address = None;
            w.pool.connecting = false;
        }
        "create-identity-pool-checking"
        | "create-identity-pool-refused"
        | "create-identity-pool-refund"
        | "create-identity-pool-not-delivered" => {
            let refused = matches!(
                which,
                "create-identity-pool-refused" | "create-identity-pool-refund"
            );
            w.unfinished_pool_identity = Some(evolite_api::UnfinishedPoolIdentity {
                exit: 3_000_000,
                name: Some("satoshi-2026".into()),
                sent_at: now - 7 * 60,
                refusal_address: "tdashevo1qz7mv8w5c3n6k2r4y9x0l5p8t2h6j3d4f7g9s2a".into(),
                refused: refused.then_some(2_742_313),
                not_delivered: which == "create-identity-pool-not-delivered",
            });
        }
        _ => {}
    }
}

/// The first id of [`contest_lines`]: the demo's History has them once.
const CLAIM_IDS: u64 = 9_021_000;

/// History's lines of the names preset (FEAT-0021 N6): the claims on @nakamoto-dev and @bob, the
/// write and the fund paid, and @bob's result, nothing paid.
fn contest_lines(
    who: &evolite_api::IdentityRef,
    now: i64,
    claim: evolite_api::Duffs,
    fund: evolite_api::Duffs,
) -> Vec<evolite_api::Activity> {
    use evolite_api::{Activity, ActivityKind, Chain, Detail, NameOutcome, Proof};
    let day = 86_400;
    let line = |id: u64, kind: ActivityKind, at: i64, amount: i64| {
        let (layers, flow) = evolite_core::layers::of(&kind, Chain::Platform);
        Activity {
            id,
            kind,
            chain: Chain::Platform,
            at,
            amount,
            proof: Proof::Proved,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers,
            flow,
            key: format!("contest-{id}"),
            detail: Some(Box::new(Detail {
                identity: Some(who.clone()),
                ..Default::default()
            })),
            note: None,
            rate: None,
            tax_transfer: false,
        }
    };
    // BUG-190/191: a claim's fee is its writes; the fund is the rest of what it took.
    let claimed = |mut line: Activity| {
        line.fee = Some(claim.saturating_sub(fund));
        line
    };
    vec![
        line(
            CLAIM_IDS,
            ActivityKind::NameContestEnded {
                label: "bob".into(),
                outcome: NameOutcome::Lost,
            },
            now - 24 * day,
            0,
        ),
        claimed(line(
            CLAIM_IDS + 1,
            ActivityKind::NameClaimed {
                label: "nakamoto-dev".into(),
            },
            now - 5 * day,
            -claim,
        )),
        claimed(line(
            CLAIM_IDS + 2,
            ActivityKind::NameClaimed {
                label: "bob".into(),
            },
            now - 38 * day,
            -claim,
        )),
    ]
}

/// What a `tokens-*` preset does to the snapshot.
fn tokens_state(s: &mut evolite_api::Snapshot, name: &str) {
    use evolite_api::tokens::{SearchState, TokenSearch};
    if name == "tokens-mainnet" {
        s.network = evolite_api::Network::Mainnet;
        return;
    }
    let Some(w) = s.wallet.as_mut() else {
        return;
    };
    // M1 (FEAT-0019 T-3, the designer): the owner destroys frozen funds alone, so «Destroy»
    // stands beside «Unfreeze» in the frozen row; the demo gives it to a group.
    // FEAT-0019, the names: the demo locks them for good; here this identity may change them.
    if name == "tokens-names"
        && let Some(page) = w.tokens.page.as_mut()
    {
        for s in &mut page.settings {
            if s.field == evolite_api::tokens::SettingField::Names {
                s.never = false;
                s.change = evolite_api::tokens::Right::Alone;
                s.change_who = evolite_api::tokens::Right::Alone;
                s.makers = evolite_api::tokens::Taker::ContractOwner;
                s.admins = evolite_api::tokens::Taker::ContractOwner;
            }
        }
    }
    if name == "tokens-page-destroy"
        && let Some(page) = w.tokens.page.as_mut()
    {
        for (action, right) in &mut page.actions {
            if *action == evolite_api::tokens::IssuerAction::DestroyFrozen {
                *right = evolite_api::tokens::Right::Alone;
            }
        }
    }
    // K1u (FEAT-0019 T-2): what a claim pays is not known before it.
    if name == "tokens-claim-unknown" {
        for row in &mut w.tokens.rows {
            if let Some(p) = row.claims.as_mut().and_then(|c| c.perpetual.as_mut()) {
                p.at_least = None;
            }
        }
        return;
    }
    // T6b: the wallet's password asked as for every spend with «Password for every send» on.
    if name == "tokens-review" {
        w.password_each_send = true;
        return;
    }
    // T6 refused: the identity's credits do not cover the fee.
    if name == "tokens-send-credits" {
        w.balances.credits = 10;
        return;
    }
    let at = chrono::Local::now().timestamp();
    let first = |state| TokenSearch {
        state,
        stage: evolite_api::tokens::SearchStage::Contracts,
        checked: 1_840,
        total: Some(4_175),
        left_secs: Some(60),
        done: None,
        failed: false,
    };
    match name {
        "tokens-first-search" => {
            w.tokens.rows.clear();
            w.tokens.search = first(SearchState::Running);
        }
        // BUG-132: the first pass before the network's count is known.
        "tokens-first-count" => {
            w.tokens.rows.clear();
            w.tokens.search = TokenSearch {
                total: None,
                left_secs: None,
                ..first(SearchState::Running)
            };
        }
        // F: after the contracts, the balances of what was found, identity by identity.
        "tokens-balances" => {
            w.tokens.rows.clear();
            w.tokens.search = TokenSearch {
                stage: evolite_api::tokens::SearchStage::Balances { identity: 3, of: 8 },
                checked: 4_175,
                total: Some(4_175),
                left_secs: None,
                ..first(SearchState::Running)
            };
        }
        // A repeated search: the list stays, and it is not called the first (design review).
        "tokens-again" => {
            let done = w.tokens.search.done;
            w.tokens.search = TokenSearch {
                done,
                ..first(SearchState::Running)
            };
        }
        // T4 before the network's count is known: no total made up.
        "tokens-no-network-first" => {
            w.tokens.rows.clear();
            w.tokens.search = TokenSearch {
                total: None,
                left_secs: None,
                ..first(SearchState::NoNetwork)
            };
        }
        "tokens-pause" => {
            w.tokens.rows.clear();
            w.tokens.search = first(SearchState::Paused { until: at + 42 });
        }
        "tokens-no-network" => {
            w.tokens.rows.clear();
            w.tokens.search = TokenSearch {
                left_secs: None,
                ..first(SearchState::NoNetwork)
            };
        }
        "tokens-refresh-failed" => {
            w.tokens.search.state = SearchState::NoNetwork;
            w.tokens.search.failed = true;
        }
        "tokens-empty" => w.tokens.rows.clear(),
        // BUG-230 AC-T7: every token hidden on the shown identity: «No tokens» and, under it,
        // «Hidden tokens (N)».
        "tokens-all-hidden" => {
            let rows = std::mem::take(&mut w.tokens.rows);
            w.tokens.hidden = rows
                .into_iter()
                .map(|r| evolite_api::tokens::HiddenToken { info: r.info })
                .collect();
        }
        _ => {}
    }
}

/// The demo's folder of files set aside (BUG-256): a path of this system, never written to.
pub(crate) fn demo_set_aside_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("evolite-demo")
}
