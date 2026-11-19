//! TECH-spend-flow: what a screen forgets, said once. Every state that holds a paid window
//! ([`crate::components::spend_flow::SpendFlow`]) or anything typed of one wallet implements
//! [`ScreenState`], and `App::screen_states` lists them all: the lock, a wallet switch and an
//! identity switch call [`ScreenState::forget`] through that one list (the two hand-kept lists
//! of `app.rs` had drifted apart: BUG-214's Platform address top-up was forgotten on a lock
//! but not on a switch).

/// Why a screen forgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The wallet locked: what was typed for a spend goes, and the windows wait for nothing.
    Lock,
    /// Another wallet opened: nothing of the last one is shown in it.
    WalletSwitch,
    /// The wallet acts as another identity: a window that was asked for the last one is over
    /// (the core still finishes what it was asked, and History shows it).
    IdentitySwitch,
}

/// A screen's state that forgets (TECH-spend-flow AC-T2).
pub trait ScreenState {
    fn forget(&mut self, why: Why);

    /// The paid windows' flows this state holds now (the guard tests: App lists every one).
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    fn flows(&mut self) -> Vec<&mut dyn crate::components::spend_flow::AnyFlow>;
}
