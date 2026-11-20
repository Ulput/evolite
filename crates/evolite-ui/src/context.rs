//! What a screen gets for one frame: the last snapshot from the core, the time, a place for
//! commands to the core and a way to switch screens. Screens never talk to the core directly.

use crate::screens::Screen;
use crate::theme::Breakpoint;
use chrono::{DateTime, Local};
use evolite_api::{Command, Duffs, Snapshot, Source, Wallet};

/// The Send screen filled in by another screen: from where, to whom, and how much when known.
pub struct Pay {
    pub source: Source,
    pub to: String,
    pub amount: Option<Duffs>,
}

pub struct Ctx<'a> {
    pub snap: &'a Snapshot,
    pub now: DateTime<Local>,
    /// The window's width class this frame (responsive spec R2): what screens go by for one
    /// column or two; the app's frame for the rail and the margins.
    pub bp: Breakpoint,
    commands: &'a mut Vec<Command>,
    go: &'a mut Option<Screen>,
    /// The Send screen asked for with a source and a recipient filled in.
    pay: Option<Pay>,
    /// The Identity screen asked for with "Top up the identity" open.
    top_up: bool,
    /// FEAT-0020 K7: the keys screen asked for with «For DashPay» chosen.
    enable_dashpay: bool,
    /// The Identity screen asked for with a claim's contest card open (FEAT-0021 N5): the
    /// identity that claimed and the label.
    contest: Option<(String, String)>,
    /// FEAT-0015 stage 2 (frame 14): «Check and top up» on Send — the review of a funding of the
    /// Platform address for this amount, over the screen.
    fund: Option<evolite_api::Duffs>,
    /// Something about the wallets of this computer the app does across frames.
    wallets: Option<WalletRequest>,
    /// BUG-169: where «Cancel», «‹ {place}», Esc and the mouse's «back» on a returning screen
    /// lead — the screen it was entered from, or Overview when there is none to go back to.
    /// Set by `App` each frame.
    pub back: Screen,
    /// [`Self::go_back`] was asked this frame.
    go_back: bool,
    /// «Features for advanced users» on (FEAT-0040…0046): set by `App` each frame.
    advanced: bool,
    /// BUG-248: the open wallet has DASH locked on Core for an unfinished operation (an
    /// identity, its top-up, a funding of the Platform address, a shielding), as the core says — also while the sync has not read it yet, when [`Self::snap`] holds it
    /// back ([`crate::text::without_unknown_locks`]). Only the deletion's warning reads it: a
    /// needless warning there costs little, a missing one money. Set by `App` each frame.
    pub unfinished_lock: bool,
}

/// what a screen asks of the wallets that takes more than one command.
#[derive(Clone, Debug, PartialEq)]
pub enum WalletRequest {
    /// "Add wallet" → this onboarding (none: the welcome screen's choice), once the core
    /// has made room for the new wallet.
    Add(Option<crate::screens::onboarding::Flow>),
    /// "Manage wallets": Settings → Wallets.
    Manage,
    /// "Forgot the password?" for a wallet that is not open: it is opened (its unlock screen)
    /// with the deletion asking no password. Settings deletes another wallet in place, by its
    /// own password (R-28).
    ForgotOther(String),
    /// Another wallet chosen (the menu, Settings): "Open "…"" asks its password over the
    /// screen (Figma 29).
    Open(String),
}

/// The network this app runs on: the one it was launched on (FEAT-0037), known before any
/// wallet is open. The screens' one way to it (BUG-211): every check of a typed address takes it.
pub fn network() -> evolite_api::Network {
    evolite_core::network::current()
}

impl<'a> Ctx<'a> {
    /// [`network`], for a screen that has the context at hand.
    pub fn network(&self) -> evolite_api::Network {
        network()
    }

    pub fn new(
        snap: &'a Snapshot,
        commands: &'a mut Vec<Command>,
        go: &'a mut Option<Screen>,
    ) -> Self {
        Self {
            snap,
            now: Local::now(),
            bp: Breakpoint::default(),
            commands,
            go,
            pay: None,
            top_up: false,
            enable_dashpay: false,
            contest: None,
            fund: None,
            wallets: None,
            back: Screen::Overview,
            go_back: false,
            advanced: false,
            unfinished_lock: false,
        }
    }

    /// Whether «Features for advanced users» is on (FEAT-0040…0046): the one question a screen
    /// asks before it shows what only that switch shows. Off by default.
    #[allow(
        dead_code,
        reason = "the screens of FEAT-0040…0046 ask it; the first one takes this away"
    )]
    pub fn advanced(&self) -> bool {
        self.advanced
    }

    /// `App` says it once a frame, from the app's settings.
    pub fn show_advanced(&mut self, on: bool) {
        self.advanced = on;
    }

    /// Back to [`Self::back`] (BUG-169); `App` checks the place is still there, else Overview.
    pub fn go_back(&mut self) {
        self.go_back = true;
    }

    /// Whether [`Self::go_back`] was asked this frame.
    pub fn take_go_back(&mut self) -> bool {
        std::mem::take(&mut self.go_back)
    }

    pub fn wallets(&mut self, request: WalletRequest) {
        self.wallets = Some(request);
    }

    /// What [`Self::wallets`] asked for this frame.
    pub fn take_wallets(&mut self) -> Option<WalletRequest> {
        self.wallets.take()
    }

    /// The wallet the screens show. Screens with the sidebar only open with one.
    pub fn wallet(&self) -> &'a Wallet {
        self.snap
            .wallet
            .as_ref()
            .expect("wallet screens open only with a wallet")
    }

    /// Sent to the core after the frame.
    pub fn send(&mut self, command: Command) {
        self.commands.push(command);
    }

    pub fn go(&mut self, screen: Screen) {
        *self.go = Some(screen);
    }

    /// Opens the Send screen paying from `source` to `to` (empty: the person types it), e.g.
    /// "Withdraw to Core" — credits to the wallet's own Core address.
    pub fn pay(&mut self, source: Source, to: String) {
        self.pay = Some(Pay {
            source,
            to,
            amount: None,
        });
        self.go(Screen::Send);
    }

    /// [`Self::pay`] with the amount too: "Send again" from an operation's card.
    pub fn pay_again(&mut self, source: Source, to: String, amount: Duffs) {
        self.pay = Some(Pay {
            source,
            to,
            amount: Some(amount),
        });
        self.go(Screen::Send);
    }

    /// What [`Self::pay`] asked for this frame.
    pub fn take_pay(&mut self) -> Option<Pay> {
        self.pay.take()
    }

    /// Opens Identity with "Top up the identity" on it (as "Top up the identity" on Overview does).
    pub fn top_up(&mut self) {
        self.top_up = true;
        self.go(Screen::Identity);
    }

    /// Whether [`Self::top_up`] was asked this frame.
    pub fn take_top_up(&mut self) -> bool {
        std::mem::take(&mut self.top_up)
    }

    /// FEAT-0020 K7: opens the identity's keys with DashPay's pair to add (Contacts'
    /// «Enable DashPay…»).
    pub fn enable_dashpay(&mut self) {
        self.enable_dashpay = true;
        self.go(Screen::IdentityKeys);
    }

    /// Whether [`Self::enable_dashpay`] was asked this frame.
    pub fn take_enable_dashpay(&mut self) -> bool {
        std::mem::take(&mut self.enable_dashpay)
    }

    /// Opens Identity with the contest card of `identity`'s claim on `label` (FEAT-0021): one way
    /// in for every entry (AC-T21) — the names, Overview's notice, History, the result's
    /// notification and «Add a name». Another identity claimed it: the wallet acts as that one.
    pub fn open_contest(&mut self, identity: &str, label: &str) {
        self.contest = Some((identity.to_owned(), label.to_owned()));
        self.go(Screen::Identity);
    }

    /// What [`Self::open_contest`] asked for this frame.
    pub fn take_contest(&mut self) -> Option<(String, String)> {
        self.contest.take()
    }

    /// Opens «Check the top-up» (22b) of a funding of the Platform address that gets at least
    /// `amount` (FEAT-0015 stage 2, frame 14).
    pub fn fund_address(&mut self, amount: evolite_api::Duffs) {
        self.fund = Some(amount);
    }

    /// What [`Self::fund_address`] asked for this frame.
    pub fn take_fund(&mut self) -> Option<evolite_api::Duffs> {
        self.fund.take()
    }
}
