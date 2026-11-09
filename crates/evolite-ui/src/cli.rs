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

/// Where the real core keeps its files: `--data-dir`, else the OS's per-user data folder —
/// `evolite` for testnet, `evolite-mainnet` for mainnet, so the two never share a folder (inside
/// any folder the core keeps each network's files apart as well).
fn data_dir(arg: Option<&str>, network: evolite_api::Network) -> PathBuf {
    if let Some(dir) = arg {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("APPDATA")
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
        .unwrap_or_else(|| PathBuf::from("."));
    match network {
        evolite_api::Network::Testnet => base.join("evolite"),
        evolite_api::Network::Mainnet => base.join("evolite-mainnet"),
    }
}

/// What the app starts with.
pub struct Launch {
    pub core: CoreChoice,
    pub screen: Screen,
    pub dark: bool,
    /// `--lang <code>`: the interface language for this run (screenshots in every language).
    pub lang: Option<crate::i18n::Lang>,
    /// `--network mainnet`; testnet without it. With the demo, only to see how mainnet looks.
    pub network: evolite_api::Network,
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
    /// `connect`, `sign`, `exchange`: the Apps screen with that request open.
    pub example_link: Option<&'static str>,
    /// `--screenshot <file.png>`: save the window after a few frames and quit.
    pub screenshot: Option<String>,
    /// `--open <screen>`: the real core with a screen open, for screenshots of the real wallet;
    /// the vault is opened with `EVOLITE_SHOT_PASSWORD` when set, and the shot waits for the
    /// sync. Nothing is sent that changes the wallet.
    pub real_shot: bool,
    /// `--window <W>x<H>`: the window's size in points, for screenshots of small screens.
    pub window: Option<[f32; 2]>,
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
        let arg = |name: &str| {
            args.iter()
                .position(|a| a == name)
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
        };
        // `--network mainnet`: real money, only when asked for at the start; testnet otherwise.
        let network = match arg("--network") {
            Some("mainnet") => evolite_api::Network::Mainnet,
            _ => evolite_api::Network::Testnet,
        };
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
    "overview", "hidden", "units", "remind", "wallet-menu", "wallet-switch", "overview-syncing",
    "activity", "activity-card", "activity-filters", "activity-filters-empty",
    "activity-card-self", "activity-card-received",
    "activity-notes", "activity-search-note", "activity-notes-unreadable", "activity-card-note",
    "activity-card-add-note", "activity-card-note-edit", "activity-card-note-error",
    "send-note", "send-note-limit", "send-confirm-note", "delete-notes",
    "send", "send-confirm", "send-contact", "send-book", "send-syncing", "send-qr-menu",
    "send-qr-filled", "send-qr-error", "send-confirm-link",
    "receive",
    "identity", "identity-unfinished", "identity-created", "top-up-stopped", "top-up-awaiting",
    "edit-profile",
    "top-up", "add-name",
    "contacts", "add-contact", "contact-profile", "my-qr",
    "create-identity", "creating", "create-identity-nameless", "create-identity-confirm",
    "create-identity-short", "create-identity-unchecked",
    "shielded", "shield", "shield-core", "unshield", "unshield-identity", "pool-address",
    "apps", "connect", "sign", "exchange",
    "collection", "gift",
    "probe",
    "settings", "phrase", "check-phrase", "appearance", "appearance-rate", "change-password",
    "security", "backup", "notifications", "own-node", "nodes", "nodes-dapi", "about",
    "about-end", "report", "report-letter", "delete", "resync", "language", "wallets",
    "wallet-rename", "wallet-more",
    "welcome", "elsewhere", "forgot", "wallet-unlock", "wallet-add",
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
        Some("connect") => Some("dash-key:yappr"),
        Some("sign") => Some("dash-st:jembe"),
        Some("exchange") => Some("dash-kx:jembe"),
        _ => None,
    };
    let screen = match (&onboarding, preset) {
        (Some(o), _) => Screen::Onboarding(o.flow),
        // `wallet-menu`: Overview with the wallet menu open (Figma 26).
        // `wallet-switch`: "Open "Savings"" over Overview (Figma 29).
        (
            None,
            Some(
                "overview" | "hidden" | "units" | "remind" | "wallet-menu" | "wallet-switch"
                | "wallet-switch-wait" | "overview-syncing",
            ),
        ) => Screen::Overview,
        (
            None,
            Some(
                "activity"
                | "activity-card"
                | "activity-filters"
                | "activity-filters-empty"
                | "activity-card-self"
                | "activity-card-received"
                | "activity-notes"
                | "activity-search-note"
                | "activity-notes-unreadable"
                | "activity-card-note"
                | "activity-card-add-note"
                | "activity-card-note-edit"
                | "activity-card-note-error",
            ),
        ) => Screen::Activity,
        (
            None,
            Some(
                "send" | "send-confirm" | "send-contact" | "send-book" | "send-syncing"
                | "send-note" | "send-note-limit" | "send-confirm-note" | "send-qr-menu"
                | "send-qr-filled" | "send-qr-error" | "send-confirm-link",
            ),
        ) => Screen::Send,
        (None, Some("receive")) => Screen::Receive,
        (
            None,
            Some(
                "identity"
                | "identity-unfinished"
                | "identity-created"
                | "top-up-stopped"
                | "top-up-awaiting"
                | "edit-profile"
                | "top-up"
                | "add-name",
            ),
        ) => Screen::Identity,
        (None, Some("contacts" | "add-contact" | "contact-profile" | "my-qr")) => Screen::Contacts,
        (
            None,
            Some(
                "create-identity"
                | "creating"
                | "create-identity-nameless"
                | "create-identity-confirm"
                | "create-identity-short"
                | "create-identity-unchecked",
            ),
        ) => Screen::CreateIdentity,
        (
            None,
            Some(
                "shielded" | "shield" | "shield-core" | "unshield" | "unshield-identity"
                | "pool-address",
            ),
        ) => Screen::Shielded,
        (None, Some("apps" | "connect" | "sign" | "exchange")) => Screen::Apps,
        (None, Some("collection" | "gift")) => Screen::Collection,
        (None, Some("probe")) => Screen::Probe,
        (
            None,
            Some(
                "settings" | "phrase" | "check-phrase" | "appearance" | "change-password"
                | "security" | "backup" | "notifications" | "own-node" | "appearance-rate"
                | "nodes" | "nodes-dapi" | "about" | "about-end" | "report" | "report-letter"
                | "delete" | "resync" | "language" | "wallets" | "wallet-rename" | "wallet-more"
                | "delete-notes",
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
        Launch::from_args(&args)
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

    #[cfg(feature = "dev-tools")]
    #[test]
    fn screens_and_presets() {
        let l = launch("--screen shield --theme light");
        assert_eq!(l.screen, Screen::Shielded);
        assert!(l.dev.preset("shield") && !l.dev.preset("gift") && !l.dark);
        let l = launch("--screen sign");
        assert_eq!(
            (l.screen, l.dev.example_link),
            (Screen::Apps, Some("dash-st:jembe"))
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
