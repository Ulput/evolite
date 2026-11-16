//! Screens from Figma "evolite — UI/UX", page Screens, one file each, plus a script probe
//! that is not in the design (`dev-tools` only). Here: which screen is open and the menu items.

pub mod app_requests;
pub mod apps;
pub mod collection;
pub mod connect2;
pub mod contacts;
pub mod create_identity;
pub mod delete_wallet;
pub mod fund_address;
pub mod identity;
pub mod onboarding;
pub mod pause;
pub mod receive;
pub mod send;
pub mod settings;
pub mod shielded;
pub mod tokens;
pub mod unlock;

pub mod activity;
pub mod activity_card;
pub mod activity_filters;
mod elsewhere;
mod overview;
#[cfg(feature = "dev-tools")]
mod probe;
pub(crate) mod sidebar;
pub mod switch_wallet;
pub mod wallets;
pub(crate) mod welcome;

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
    /// FEAT-0019: the identity's Dash Platform tokens; on mainnet it only explains (R-9).
    Tokens,
    /// FEAT-0019 T-3: one token's page (M1), entered from Tokens; the sidebar keeps Tokens lit.
    TokenPage,
    Settings,
    /// 07: opened from Identity; the sidebar keeps Identity lit.
    CreateIdentity,
    /// FEAT-0020 K1: an identity's keys, opened from Identity, Settings or Apps; the sidebar keeps
    /// Identity lit, «‹ {place}» goes back to where it was opened from (AC-T17).
    IdentityKeys,
    /// O1–O6, without the sidebar.
    Onboarding(onboarding::Flow),
    /// The script check (Arabic and Hindi shaping), `--screen probe` with `dev-tools`.
    #[cfg(feature = "dev-tools")]
    Probe,
}

pub const NAV: [(Screen, &str, &str); 10] = [
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
    // Last, after «Collection» (designer-47: new items go to the end).
    (Screen::Tokens, "token", tr!("Tokens")),
];

/// What a menu item needs of the network matrix (FEAT-0037); `None`: every network has it.
pub fn feature_of(screen: Screen) -> Option<evolite_api::features::Feature> {
    use evolite_api::features::Feature;
    match screen {
        Screen::Tokens | Screen::TokenPage => Some(Feature::Tokens),
        Screen::Apps => Some(Feature::Apps),
        Screen::Collection => Some(Feature::Collection),
        _ => None,
    }
}

/// Whether `screen` is in the menu on `network`: what the network matrix lets in.
pub fn shown_on(screen: Screen, network: evolite_api::Network) -> bool {
    feature_of(screen).is_none_or(|f| evolite_api::features::on(f, network))
}

/// A screen's name as the menu says it (the key): what «‹ {place}» names (BUG-169). Settings by
/// its button; «Create an identity» is Identity's, as the menu keeps it lit.
pub fn place_name(screen: Screen) -> &'static str {
    match screen {
        Screen::Settings => tr!("Settings"),
        Screen::CreateIdentity | Screen::IdentityKeys => tr!("Identity"),
        Screen::TokenPage => tr!("Tokens"),
        _ => NAV
            .iter()
            .find(|(s, ..)| *s == screen)
            .map_or(tr!("Overview"), |(_, _, name)| name),
    }
}

pub(crate) use crate::components::txt;

/// SPV Б2 and item 3: an asset lock of `lock` duffs its coins cannot make (more inputs than the
/// network takes, or final coins short), said under the amount in the danger colour, in the
/// core's own words: the form refuses it before «Check» as the core would before the password.
/// `true` when refused.
pub(crate) fn lock_refused_line(
    ui: &mut egui::Ui,
    p: &crate::theme::Palette,
    fees: &evolite_api::Fees,
    lock: Option<evolite_api::Duffs>,
) -> bool {
    let Some(words) = lock.and_then(|d| evolite_core::lock_words::lock_refusal(fees.lock_coins(d)))
    else {
        return false;
    };
    let small = crate::theme::type_scale::SMALL;
    ui.add(
        crate::components::para(&words, small.size, small.weight, p.danger).line_height(small.line),
    );
    true
}

/// SPV item 2a: in «Check …» of a payment that spends Core coins, over an hour without a new
/// block — a line in the warning colour, not a ban: the button stays, nothing more is asked
/// (the analyst).
pub(crate) fn stale_core_line(
    ui: &mut egui::Ui,
    p: &crate::theme::Palette,
    sync: &evolite_api::SyncInfo,
) {
    let Some(minutes) = sync.stale_minutes.filter(|m| *m >= 60) else {
        return;
    };
    let said = crate::text::plural_in(
        minutes / 60,
        tr!(
            "No new blocks for over {} hour: the balance and received coins may be out of date. You can still pay."
        ),
        tr!(
            "No new blocks for over {} hours: the balance and received coins may be out of date. You can still pay."
        ),
    );
    ui.add(
        crate::components::para(&said, 13.0, crate::theme::Weight::Regular, p.accent)
            .line_height(18.0),
    );
}
