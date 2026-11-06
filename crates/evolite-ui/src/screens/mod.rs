//! Screens from Figma "evolite — UI/UX", page Screens, one file each, plus a script probe
//! that is not in the design (`dev-tools` only). Here: which screen is open and the menu items.

pub mod app_requests;
pub mod apps;
pub mod collection;
pub mod contacts;
pub mod create_identity;
pub mod delete_wallet;
pub mod identity;
pub mod onboarding;
pub mod pause;
pub mod receive;
pub mod send;
pub mod settings;
pub mod shielded;
pub mod unlock;

pub mod activity;
mod elsewhere;
mod overview;
#[cfg(feature = "dev-tools")]
mod probe;
mod sidebar;
pub mod switch_wallet;
pub mod wallets;
mod welcome;

pub use activity::activity;
#[cfg(feature = "dev-tools")]
pub use activity::open_card;
pub use elsewhere::elsewhere;
pub use overview::overview;
#[cfg(feature = "dev-tools")]
pub use probe::probe;
pub use sidebar::sidebar;
pub use welcome::welcome;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Screen {
    Welcome,
    /// The wallet is saved here, its keys are closed.
    Unlock,
    Overview,
    Send,
    Receive,
    Activity,
    Identity,
    /// 19: DashPay contacts and requests.
    Contacts,
    Shielded,
    Apps,
    Collection,
    Settings,
    /// 07: opened from Identity; the sidebar keeps Identity lit.
    CreateIdentity,
    /// O1–O6, without the sidebar.
    Onboarding(onboarding::Flow),
    /// The script check (Arabic and Hindi shaping), `--screen probe` with `dev-tools`.
    #[cfg(feature = "dev-tools")]
    Probe,
}

pub const NAV: [(Screen, &str, &str); 9] = [
    (Screen::Overview, "home", tr!("Overview")),
    (Screen::Send, "send", tr!("Send")),
    (Screen::Receive, "receive", tr!("Receive")),
    (Screen::Activity, "activity", tr!("History")),
    // A Dash Platform term: it stays where a language has no word for it (Russian).
    (Screen::Identity, "identity", tr!("Identity")),
    (Screen::Contacts, "contacts", tr!("Contacts")),
    (Screen::Shielded, "shielded", tr!("Private pool")),
    (Screen::Apps, "apps", tr!("Apps")),
    (Screen::Collection, "nft", tr!("Collection")),
];

pub(crate) use crate::components::txt;
