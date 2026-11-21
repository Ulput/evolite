//! The command line: the data folder, the network, the theme and the language. With the
//! `dev-tools` feature also which core and which screen to open (screenshots and checks) and a
//! screenshot to save; without it those flags are not read at all.

use crate::screens::Screen;
#[cfg(feature = "dev-tools")]
use crate::screens::onboarding::{Flow, Onboarding, Step};
use std::path::PathBuf;

#[derive(Clone)]
pub enum CoreChoice {
    /// The Figma example wallet with a staged sync: screenshots and the demo.
    #[cfg(demo_core)]
    Mock,
    /// dash-spv on testnet, files under this directory.
    Real(PathBuf),
}

/// Where pictures drawn by the renderer are kept between runs: the OS's per-user cache folder
/// (it may be emptied at any time; everything there is drawn again).
pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let home = PathBuf::from(h);
                if cfg!(target_os = "macos") {
                    home.join("Library").join("Caches")
                } else {
                    home.join(".cache")
                }
            })
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("evolite")
}

/// The cache of one data folder: pictures one wallet saw are not kept for another
/// folder, and the core empties it when the wallet is deleted. The demo has its own.
pub fn cache_for(core: &CoreChoice) -> PathBuf {
    use sha2::{Digest, Sha256};
    match core {
        #[cfg(demo_core)]
        CoreChoice::Mock => cache_dir().join("demo"),
        CoreChoice::Real(dir) => {
            let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
            let hash = Sha256::digest(dir.to_string_lossy().as_bytes());
            let short: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
            cache_dir().join(format!("data-{short}"))
        }
    }
}

/// The diagnostic journal's folder: `logs` in the data folder, the demo's in its cache.
pub fn logs_for(core: &CoreChoice) -> PathBuf {
    match core {
        #[cfg(demo_core)]
        CoreChoice::Mock => cache_dir().join("demo").join("logs"),
        CoreChoice::Real(dir) => dir.join("logs"),
    }
}

/// A network's own data folder beside the other's, in the OS's per-user data folder (FEAT-0037: the
/// switch's target).
pub(crate) fn network_dir(network: evolite_api::Network) -> PathBuf {
    data_dir(None, network)
}

/// Where the real core keeps its files: `--data-dir`, else the OS's per-user data folder —
/// `evolite` for testnet, `evolite-mainnet` for mainnet, so the two never share a folder (inside
/// any folder the core keeps each network's files apart as well).
fn data_dir(arg: Option<&str>, network: evolite_api::Network) -> PathBuf {
    if let Some(dir) = arg {
        return PathBuf::from(dir);
    }
    let base = base_dir();
    match network {
        evolite_api::Network::Testnet => base.join("evolite"),
        evolite_api::Network::Mainnet => base.join("evolite-mainnet"),
    }
}

/// The OS's per-user data folder, which holds the networks' folders and the app's common one.
pub(crate) fn base_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let home = PathBuf::from(h);
                if cfg!(target_os = "macos") {
                    home.join("Library").join("Application Support")
                } else {
                    home.join(".local").join("share")
                }
            })
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

/// What the app starts with.
pub struct Launch {
    pub core: CoreChoice,
    pub screen: Screen,
    pub dark: bool,
    /// `--lang <code>`: the interface language for this run (screenshots in every language).
    pub lang: Option<crate::i18n::Lang>,
    /// `--network`, else the last network chosen, else testnet (FEAT-0037, AC-T6а).
    pub network: evolite_api::Network,
    /// The app's common folder: the last network and the app's settings (FEAT-0037).
    pub app_dir: PathBuf,
    /// Started by a switch of network (FEAT-0037): a held folder is said as that network open
    /// in another window.
    pub switched: bool,
    /// The old window's place and size, `x, y, w, h` in points, after a switch (AC-T6ж).
    pub window_at: Option<[f32; 4]>,
    /// What `dev-tools` reads besides: the screen preset, the screenshot, the window.
    #[cfg(feature = "dev-tools")]
    pub dev: DevLaunch,
}

/// The flags of the `dev-tools` build.
#[cfg(feature = "dev-tools")]
#[derive(Default)]
pub struct DevLaunch {
    /// `--screen`'s value: some screens open in a state from Figma (a dialog, a filled form).
    pub preset: Option<String>,
    /// Onboarding steps open filled in as in Figma.
    pub onboarding: Option<Onboarding>,
    /// `connect`: the Apps screen with an App Connect v2 request open.
    pub example_link: Option<&'static str>,
    /// `--screenshot <file.png>`: save the window after a few frames and quit.
    pub screenshot: Option<String>,
    /// `--open <screen>`: the real core with a screen open, for screenshots of the real wallet;
    /// the vault is opened with `EVOLITE_SHOT_PASSWORD` when set, and the shot waits for the
    /// sync. Nothing is sent that changes the wallet.
    pub real_shot: bool,
    /// `--window <W>x<H>`: the window's size in points, for screenshots of small screens.
    pub window: Option<[f32; 2]>,
    /// `--stop-after <step>`: the real core ends the process after that step of an operation,
    /// for QA to continue it every time (BUG-160). `main` knows the steps.
    pub stop_after: Option<String>,
}

#[cfg(feature = "dev-tools")]
impl DevLaunch {
    /// Opened with `--screen <name>` (or `--open <name>`).
    pub fn preset(&self, name: &str) -> bool {
        self.preset.as_deref() == Some(name)
    }
}

impl Launch {
    pub fn from_args(args: &[String]) -> Self {
        Self::from_args_with(args, crate::network_choice::read_last)
    }

    /// [`Launch::from_args`] with the last network chosen read by `last` (a test reads none).
    pub fn from_args_with(
        args: &[String],
        last: impl Fn(&std::path::Path) -> Option<evolite_api::Network>,
    ) -> Self {
        let arg = |name: &str| {
            args.iter()
                .position(|a| a == name)
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
        };
        // `--network`, else the last network chosen, else testnet (FEAT-0037, the customer's М3).
        let app_dir = crate::network_choice::app_dir(arg("--data-dir"), &base_dir());
        let network = crate::network_choice::start_network(arg("--network"), last(&app_dir));
        let real = CoreChoice::Real(data_dir(arg("--data-dir"), network));
        #[cfg(feature = "dev-tools")]
        let (core, screen, dev) = dev_args(args, &arg, real);
        #[cfg(all(not(feature = "dev-tools"), feature = "real"))]
        let (core, screen) = (real, Screen::Welcome);
        // Built without the real core: the demo is all there is.
        #[cfg(all(not(feature = "dev-tools"), not(feature = "real")))]
        let (core, screen) = {
            drop(real);
            (CoreChoice::Mock, Screen::Welcome)
        };
        Self {
            core,
            screen,
            dark: arg("--theme") != Some("light"),
            lang: arg("--lang").and_then(crate::i18n::Lang::from_code),
            network,
            app_dir,
            switched: args.iter().any(|a| a == crate::network_choice::SWITCHED),
            window_at: arg(crate::network_choice::WINDOW_AT)
                .and_then(crate::network_choice::window_at),
            #[cfg(feature = "dev-tools")]
            dev,
        }
    }
}

/// Every `--screen` preset (`dev_args` here; `app/dev.rs`, `apply_presets`): the Tab sweep goes
/// through them all, and a test checks that no preset named in the code is missing here.
/// (Only the tests read it.)
#[cfg(all(test, feature = "dev-tools"))]
#[rustfmt::skip]
pub const PRESETS: &[&str] = &[
    "create", "verify", "password", "restore", "dashpay", "sync", "sync-nodes",
    "overview", "hidden", "units", "remind", "wallet-menu", "wallet-switch", "overview-syncing", "sync-platform", "sync-platform-short", "sync-stale-30", "sync-stale-60",
    "activity", "activity-card", "activity-filters", "activity-filters-empty", "activity-long-note",
    "activity-card-self", "activity-card-received",
    "activity-card-name", "activity-card-withdrawal-waiting", "activity-card-withdrawal-paid",
    "activity-card-payout", "activity-card-lock-not-added", "activity-card-shield",
    "activity-card-old", "activity-card-pool-sent", "activity-card-pool-received",
    "activity-card-top-up", "activity-card-lock-used", "activity-card-lock-partly",
    "activity-card-lock-not-received", "activity-card-lock-unknown",
    "activity-card-long-title", "activity-fee-unknown",
    "activity-notes", "activity-search-note", "activity-notes-unreadable", "activity-card-note",
    "activity-card-add-note", "activity-card-note-edit", "activity-card-note-error",
    "activity-card-tax-transfer", "activity-card-tax-received", "activity-card-tax-explainer",
    "activity-card-tax-error",
    "history-set-aside", "top-ups-set-aside", "overview-set-aside", "names-set-aside",
    "apps-set-aside", "contacts-set-aside", "address-book-set-aside",
    "send-note", "send-note-limit", "send-confirm-note", "delete-notes",
    "send", "send-confirm", "send-confirm-wait", "send-done", "send-contact", "send-book", "names-offline", "send-syncing", "send-qr-menu",
    "send-qr-filled", "send-qr-error", "send-confirm-link",
    "send-from-address", "address-withdraw-warning", "address-withdraw-foreign", "address-top-up",
    "address-confirm",
    "core-to-platform-address", "send-fund-own",
    "address-top-up-foreign",
    "send-pool-unread", "send-pool-unknown",
    "send-fiat", "send-fiat-confirm", "send-fiat-stale",
    "send-coins", "send-coins-chosen", "send-coins-name", "send-coins-confirm", "coins-manual",
    "coins-auto", "coins-not-enough", "coins-spent", "send-lookalike", "send-confirm-tall",
    "overview-mixed", "send-mixed", "coins-mixed", "coins-mixed-refused", "activity-mixed",
    "send-unsignable", "send-unsignable-short",
    "fund-address", "fund-address-review", "fund-address-steps", "fund-address-unfinished", "fund-address-recovered",
    "fund-address-resume", "fund-address-done",
    "overview-mixed-unchecked", "send-mixed-name", "send-mixed-refused", "send-mixed-confirm",
    "send-mixed-agree", "send-mixed-changed", "send-mixed-refused-confirm",
    "receive", "receive-fiat",
    "activity-fiat", "activity-card-fiat", "activity-card-fiat-old", "activity-apps", "activity-tokens",
    "identity", "identity-unfinished", "identity-created", "top-up-stopped", "top-up-awaiting",
    "edit-profile", "edit-profile-password", "edit-profile-imgur-page",
    "top-up", "top-up-bad-amount", "top-up-many-coins", "top-up-not-final", "add-name",
    "names-contested", "contest-card", "contest-lost", "add-name-contested", "add-name-review",
    "add-name-sent", "add-name-fund-rose", "contest-lost-hidden",
    "contacts", "add-contact", "contact-profile", "my-qr", "contacts-no-encryption",
    "contacts-channel-broken", "contact-accept", "contact-resend",
    "identity-keys", "identity-key-shown", "identity-keys-no-encryption", "key-disable",
    "key-replace-main", "key-replace-dashpay", "key-add", "identity-keys-foreign",
    "key-enable-dashpay-foreign",
    "create-identity", "creating", "create-identity-nameless", "create-identity-confirm",
    "create-identity-short", "create-identity-unchecked", "create-identity-many-coins",
    "shielded", "shield", "shield-core", "unshield", "unshield-bad-amount", "unshield-identity", "pool-address",
    "apps", "connect", "connect-limits", "connect-renew", "refused-v1", "refused-stale",
    "refused-mainnet", "apps-disconnected", "apps-renew", "apps-disconnect", "apps-remove",
    "collection", "gift",
    "workshop", "workshop-new", "workshop-review", "workshop-steps", "workshop-collection",
    "tokens", "tokens-first-search", "tokens-first-count", "tokens-balances", "tokens-again", "tokens-no-network-first", "tokens-pause", "tokens-no-network", "tokens-refresh-failed",
    "tokens-empty", "tokens-all-hidden", "tokens-send", "tokens-review", "tokens-add", "tokens-mainnet",
    "tokens-send-address", "tokens-send-more", "tokens-send-credits", "tokens-send-unknown",
    "tokens-add-one", "tokens-add-many", "tokens-add-none", "tokens-add-hidden",
    "tokens-claim", "tokens-claim-unknown", "tokens-buy", "tokens-buy-changed",
    "tokens-page", "tokens-page-destroy", "tokens-names", "tokens-mint", "tokens-burn", "tokens-destroy", "tokens-suspend", "tokens-price",
    "tokens-setting",
    "probe",
    "settings", "phrase", "check-phrase", "appearance", "appearance-rate", "change-password",
    "change-password-wait",
    "security", "backup", "notifications", "advanced", "advanced-workshop", "own-node", "nodes", "nodes-dapi", "about",
    "about-end", "report", "report-letter", "delete", "resync", "language", "wallets",
    "wallet-rename", "wallet-more", "network-switch",
    "welcome", "welcome-other", "elsewhere", "forgot", "wallet-unlock", "wallet-add",
    "unlock", "unlock-warn", "unlock-wait", "unlock-wait-end", "wallet-unlock-wait",
    "wallet-switch-wait",
];

/// `--screen`, `--open`, `--demo`, `--screenshot` and `--window`: the core (the demo for a
/// screen, unless `--open` asks for the real wallet), the screen and its preset.
#[cfg(feature = "dev-tools")]
fn dev_args<'a>(
    args: &'a [String],
    arg: &dyn Fn(&str) -> Option<&'a str>,
    real: CoreChoice,
) -> (CoreChoice, Screen, DevLaunch) {
    // `--open`: a screen as `--screen` opens it, but over the real wallet in `--data-dir`.
    let open = arg("--open");
    let preset = arg("--screen").or(open);
    let onboarding = match preset {
        Some("create") => Some(Onboarding::new(Flow::Create)),
        Some("verify") => Some(Onboarding::example(Step::Verify)),
        Some("password") => Some(Onboarding::example(Step::Password)),
        Some("restore") => Some(Onboarding::example(Step::Restore)),
        Some("dashpay") => Some(Onboarding::example(Step::Found)),
        // `sync-nodes`: the sync after a restore with "About the nodes" open over it.
        Some("sync" | "sync-nodes") => Some(Onboarding::example(Step::Sync)),
        _ => None,
    };
    let example_link = match preset {
        Some("connect" | "connect-limits") => Some("dashpay://connect?v=2&n=t&r=AQ"),
        // The demo core reads these words in its links (FEAT-0023 round 4 shots).
        Some("connect-renew") => Some("dashpay://connect?v=2&n=t&r=AQ&renew"),
        Some("refused-v1") => Some("dash-key:3mJr7AoUXx2Wqd?n=t&v=1"),
        Some("refused-stale") => Some("dashpay://connect?v=2&n=t&r=AQ&stale"),
        Some("refused-mainnet") => Some("dashpay://connect?v=2&n=m&r=AQ"),
        _ => None,
    };
    let screen = match (&onboarding, preset) {
        (Some(o), _) => Screen::Onboarding(o.flow),
        // `wallet-menu`: Overview with the wallet menu open (Figma 26).
        // `wallet-switch`: "Open "Savings"" over Overview (Figma 29).
        (
            None,
            Some(
                "overview"
                | "hidden"
                | "units"
                | "remind"
                | "wallet-menu"
                | "wallet-switch"
                | "wallet-switch-wait"
                | "overview-syncing"
                | "sync-stale-30"
                | "sync-stale-60"
                | "sync-platform"
                | "sync-platform-short"
                | "overview-mixed"
                | "overview-mixed-unchecked"
                | "fund-address"
                | "fund-address-review"
                | "fund-address-steps"
                | "fund-address-unfinished"
                | "fund-address-recovered"
                | "fund-address-resume"
                | "fund-address-done",
            ),
        ) => Screen::Overview,
        (
            None,
            Some(
                "activity"
                | "activity-card"
                | "activity-mixed"
                | "activity-filters"
                | "activity-filters-empty"
                | "activity-long-note"
                | "activity-card-self"
                | "activity-card-received"
                | "activity-card-name"
                | "activity-card-withdrawal-waiting"
                | "activity-card-withdrawal-paid"
                | "activity-card-payout"
                | "activity-card-lock-not-added"
                | "activity-card-shield"
                | "activity-card-old"
                | "activity-card-pool-sent"
                | "activity-card-pool-received"
                | "activity-card-top-up"
                | "activity-card-long-title"
                | "activity-fee-unknown"
                | "activity-card-lock-used"
                | "activity-card-lock-partly"
                | "activity-card-lock-not-received"
                | "activity-card-lock-unknown"
                | "activity-fiat"
                | "activity-card-fiat"
                | "activity-card-fiat-old"
                | "activity-apps"
                | "activity-tokens"
                | "activity-notes"
                | "activity-search-note"
                | "activity-notes-unreadable"
                | "activity-card-note"
                | "activity-card-add-note"
                | "activity-card-note-edit"
                | "activity-card-note-error"
                | "activity-card-tax-transfer"
                | "activity-card-tax-received"
                | "activity-card-tax-explainer"
                | "activity-card-tax-error",
            ),
        ) => Screen::Activity,
        (
            None,
            Some(
                "send"
                | "send-confirm"
                | "send-confirm-wait"
                | "send-done"
                | "send-contact"
                | "send-book"
                | "names-offline"
                | "send-syncing"
                | "send-note"
                | "send-note-limit"
                | "send-confirm-note"
                | "send-qr-menu"
                | "send-qr-filled"
                | "send-qr-error"
                | "send-confirm-link"
                | "send-pool-unread"
                | "send-pool-unknown"
                | "send-from-address"
                | "address-withdraw-warning"
                | "address-withdraw-foreign"
                | "address-top-up"
                | "address-confirm"
                | "core-to-platform-address"
                | "send-fund-own"
                | "address-top-up-foreign"
                | "send-fiat"
                | "send-fiat-confirm"
                | "send-fiat-stale"
                | "send-coins"
                | "send-coins-chosen"
                | "send-coins-name"
                | "send-coins-confirm"
                | "coins-manual"
                | "coins-auto"
                | "coins-not-enough"
                | "coins-spent"
                | "send-lookalike"
                | "send-confirm-tall"
                | "send-mixed"
                | "send-unsignable"
                | "send-unsignable-short"
                | "send-mixed-name"
                | "send-mixed-refused"
                | "send-mixed-confirm"
                | "send-mixed-agree"
                | "send-mixed-changed"
                | "send-mixed-refused-confirm"
                | "coins-mixed"
                | "coins-mixed-refused",
            ),
        ) => Screen::Send,
        (None, Some("receive" | "receive-fiat")) => Screen::Receive,
        // BUG-204, BUG-212: a file of the wallet set aside, on the screen of its banner.
        (None, Some("history-set-aside" | "top-ups-set-aside")) => Screen::Activity,
        (None, Some("overview-set-aside")) => Screen::Overview,
        (None, Some("names-set-aside")) => Screen::Identity,
        (None, Some("apps-set-aside")) => Screen::Apps,
        (None, Some("contacts-set-aside")) => Screen::Contacts,
        (None, Some("address-book-set-aside")) => Screen::Send,
        (
            None,
            Some(
                "identity"
                | "identity-unfinished"
                | "identity-created"
                | "top-up-stopped"
                | "top-up-awaiting"
                | "edit-profile"
                | "edit-profile-password"
                | "edit-profile-imgur-page"
                | "top-up"
                | "top-up-bad-amount"
                | "top-up-many-coins"
                | "top-up-not-final"
                | "add-name"
                | "names-contested"
                | "contest-card"
                | "contest-lost"
                | "add-name-contested"
                | "add-name-review"
                | "add-name-sent"
                | "add-name-fund-rose"
                | "contest-lost-hidden",
            ),
        ) => Screen::Identity,
        (
            None,
            Some(
                "contacts"
                | "add-contact"
                | "contact-profile"
                | "my-qr"
                | "contacts-no-encryption"
                | "contacts-channel-broken"
                | "contact-accept"
                | "contact-resend",
            ),
        ) => Screen::Contacts,
        (
            None,
            Some(
                "create-identity"
                | "creating"
                | "create-identity-nameless"
                | "create-identity-confirm"
                | "create-identity-short"
                | "create-identity-unchecked"
                | "create-identity-many-coins",
            ),
        ) => Screen::CreateIdentity,
        // FEAT-0020 K1.
        (
            None,
            Some(
                "identity-keys"
                | "identity-key-shown"
                | "identity-keys-no-encryption"
                | "key-disable"
                | "key-replace-main"
                | "key-replace-dashpay"
                | "key-add"
                | "identity-keys-foreign"
                | "key-enable-dashpay-foreign",
            ),
        ) => Screen::IdentityKeys,
        (
            None,
            Some(
                "shielded"
                | "shield"
                | "shield-core"
                | "unshield"
                | "unshield-bad-amount"
                | "unshield-identity"
                | "pool-address",
            ),
        ) => Screen::Shielded,
        (
            None,
            Some(
                "apps" | "connect" | "connect-limits" | "connect-renew" | "refused-v1"
                | "refused-stale" | "refused-mainnet" | "apps-disconnected" | "apps-renew"
                | "apps-disconnect" | "apps-remove",
            ),
        ) => Screen::Apps,
        (None, Some("collection" | "gift")) => Screen::Collection,
        (
            None,
            Some(
                "tokens"
                | "tokens-first-search"
                | "tokens-first-count"
                | "tokens-balances"
                | "tokens-again"
                | "tokens-no-network-first"
                | "tokens-pause"
                | "tokens-no-network"
                | "tokens-refresh-failed"
                | "tokens-empty"
                | "tokens-send"
                | "tokens-review"
                | "tokens-add"
                | "tokens-mainnet"
                | "tokens-send-address"
                | "tokens-send-more"
                | "tokens-send-credits"
                | "tokens-send-unknown"
                | "tokens-add-one"
                | "tokens-add-many"
                | "tokens-add-none"
                | "tokens-add-hidden"
                | "tokens-all-hidden"
                | "tokens-claim"
                | "tokens-claim-unknown"
                | "tokens-buy"
                | "tokens-buy-changed",
            ),
        ) => Screen::Tokens,
        (
            None,
            Some(
                "tokens-page"
                | "tokens-page-destroy"
                | "tokens-names"
                | "tokens-mint"
                | "tokens-burn"
                | "tokens-destroy"
                | "tokens-suspend"
                | "tokens-price"
                | "tokens-setting",
            ),
        ) => Screen::TokenPage,
        (
            None,
            Some(
                "workshop"
                | "workshop-new"
                | "workshop-review"
                | "workshop-steps"
                | "workshop-collection",
            ),
        ) => Screen::Workshop,
        (None, Some("probe")) => Screen::Probe,
        (
            None,
            Some(
                "settings"
                | "phrase"
                | "check-phrase"
                | "appearance"
                | "change-password"
                | "change-password-wait"
                | "security"
                | "backup"
                | "notifications"
                | "advanced"
                | "advanced-workshop"
                | "own-node"
                | "appearance-rate"
                | "nodes"
                | "nodes-dapi"
                | "about"
                | "about-end"
                | "report"
                | "report-letter"
                | "delete"
                | "resync"
                | "language"
                | "wallets"
                | "wallet-rename"
                | "wallet-more"
                | "delete-notes"
                | "network-switch",
            ),
        ) => Screen::Settings,
        _ => Screen::Welcome,
    };
    // Screenshots of screens show the Figma example, so they take the mock.
    let core = if args.iter().any(|a| a == "--demo") || (preset.is_some() && open.is_none()) {
        CoreChoice::Mock
    } else {
        real
    };
    let dev = DevLaunch {
        preset: preset.map(str::to_owned),
        onboarding,
        example_link,
        screenshot: arg("--screenshot").map(str::to_owned),
        real_shot: open.is_some(),
        window: arg("--window").and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        }),
        stop_after: arg("--stop-after").map(str::to_owned),
    };
    (core, screen, dev)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A preset named in `dev_args` or `apply_presets` is in [`PRESETS`], which the Tab sweep
    /// goes through: a new one cannot drop out of it unseen.
    #[cfg(feature = "dev-tools")]
    #[test]
    fn every_preset_in_the_code_is_listed() {
        // Quoted names in the body of `dev_args`, and `preset("…")` in `app/dev.rs`.
        let quoted = |text: &str| -> Vec<String> {
            text.split('"')
                .skip(1)
                .step_by(2)
                .filter(|s| {
                    !s.is_empty()
                        && s.starts_with(|c: char| c.is_ascii_lowercase())
                        && s.chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                })
                .map(str::to_owned)
                .collect()
        };
        let cli = include_str!("cli.rs");
        let body = &cli[cli.find("fn dev_args").unwrap()..];
        let body = &body[..body.find("\n#[cfg(test)]").unwrap()];
        let mut named = quoted(body);
        let dev = include_str!("app/dev.rs");
        for piece in dev.split("preset(\"").skip(1) {
            named.push(piece.split('"').next().unwrap_or_default().to_owned());
        }
        let missing: Vec<&String> = named
            .iter()
            .filter(|n| !PRESETS.contains(&n.as_str()))
            .collect();
        assert!(missing.is_empty(), "not in PRESETS: {missing:?}");
        // And back: a name left in PRESETS after its preset went would open another screen
        // ("welcome" is the screen of any name not matched: `_ => Screen::Welcome`).
        let gone: Vec<&&str> = PRESETS
            .iter()
            .filter(|p| **p != "welcome")
            .filter(|p| !body.contains(&format!("\"{p}\"")) && !dev.contains(&format!("\"{p}\"")))
            .collect();
        assert!(gone.is_empty(), "in PRESETS, not in the code: {gone:?}");
        let mut sorted = PRESETS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), PRESETS.len(), "a preset listed twice");
    }

    fn launch(args: &str) -> Launch {
        let args: Vec<String> = std::iter::once("evolite")
            .chain(args.split_whitespace())
            .map(str::to_owned)
            .collect();
        // The machine's own last choice is not the test's.
        Launch::from_args_with(&args, |_| None)
    }

    #[cfg(any(feature = "real", feature = "dev-tools"))]
    #[test]
    fn the_real_core_from_its_folder() {
        assert!(matches!(launch("").core, CoreChoice::Real(_)));
        assert!(matches!(
            launch("--data-dir /tmp/evolite-x").core,
            CoreChoice::Real(dir) if dir == std::path::Path::new("/tmp/evolite-x")
        ));
    }

    /// The release reads no screenshot flags: `--screen` and `--demo` open the real wallet's
    /// usual start, as if they were not there.
    #[cfg(all(not(feature = "dev-tools"), feature = "real"))]
    #[test]
    fn the_release_ignores_the_tool_flags() {
        let l = launch("--screen send --demo --screenshot x.png");
        assert!(matches!(l.core, CoreChoice::Real(_)));
        assert_eq!(l.screen, Screen::Welcome);
    }

    /// Built without the real core and without the tools: the demo, from its welcome screen.
    #[cfg(all(not(feature = "dev-tools"), not(feature = "real")))]
    #[test]
    fn without_the_real_core_the_demo() {
        let l = launch("--screen send --data-dir /tmp/x");
        assert!(matches!(l.core, CoreChoice::Mock));
        assert_eq!(l.screen, Screen::Welcome);
    }

    #[cfg(feature = "dev-tools")]
    #[test]
    fn the_demo_for_a_screen_or_when_asked() {
        assert!(matches!(launch("--demo").core, CoreChoice::Mock));
        assert!(matches!(launch("--screen send").core, CoreChoice::Mock));
        assert!(matches!(launch("--open send").core, CoreChoice::Real(_)));
    }

    #[cfg(any(feature = "real", feature = "dev-tools"))]
    #[test]
    fn mainnet_only_when_asked_and_in_its_own_folder() {
        use evolite_api::Network;
        assert_eq!(launch("").network, Network::Testnet);
        assert_eq!(launch("--network testnet").network, Network::Testnet);
        assert_eq!(
            launch("--network main").network,
            Network::Testnet,
            "only the exact word"
        );
        let l = launch("--network mainnet");
        assert_eq!(l.network, Network::Mainnet);
        let (main, test) = match (l.core, launch("").core) {
            (CoreChoice::Real(main), CoreChoice::Real(test)) => (main, test),
            #[cfg(demo_core)]
            _ => panic!("the real core"),
        };
        assert_ne!(main, test, "mainnet never shares testnet's folder");
        assert!(main.ends_with("evolite-mainnet"));
        // A folder given by hand is used as it is; inside, each network has its own files.
        assert!(matches!(
            launch("--network mainnet --data-dir /tmp/x").core,
            CoreChoice::Real(dir) if dir == std::path::Path::new("/tmp/x")
        ));
    }

    /// BUG-160: `--stop-after asset-lock` reaches `main` with the real core; nothing without it.
    #[cfg(feature = "dev-tools")]
    #[test]
    fn a_stop_after_a_step_is_read() {
        let l = launch("--data-dir /tmp/x --stop-after asset-lock");
        assert!(matches!(l.core, CoreChoice::Real(_)));
        assert_eq!(l.dev.stop_after.as_deref(), Some("asset-lock"));
        assert_eq!(launch("--data-dir /tmp/x").dev.stop_after, None);
    }

    #[cfg(feature = "dev-tools")]
    #[test]
    fn screens_and_presets() {
        let l = launch("--screen shield --theme light");
        assert_eq!(l.screen, Screen::Shielded);
        assert!(l.dev.preset("shield") && !l.dev.preset("gift") && !l.dark);
        let l = launch("--screen connect");
        assert_eq!(
            (l.screen, l.dev.example_link),
            (Screen::Apps, Some("dashpay://connect?v=2&n=t&r=AQ"))
        );
        // A value of another flag is not a preset.
        assert!(!launch("--data-dir shield").dev.preset("shield"));
        assert_eq!(
            launch("--screen dashpay").screen,
            Screen::Onboarding(Flow::DashPay)
        );
        let l = launch("--screen identity-unfinished --lang ar");
        assert_eq!(
            (l.screen, l.lang),
            (Screen::Identity, Some(crate::i18n::Lang::Ar))
        );
    }
}
