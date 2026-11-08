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
    /// Something about the wallets of this computer the app does across frames.
    wallets: Option<WalletRequest>,
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

impl<'a> Ctx<'a> {
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
            wallets: None,
        }
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
}
