//! FEAT-0057 П3: «Credits @name: now › after» — the one line every window that writes to
//! Platform on an identity's credits shows under «Pays» (R-3.1): the same numbers the window
//! charges (R-3.2), «after» an estimate when the charge is one and a lower bound when the charge
//! is an upper bound (R-3.3); amber with «Few credits will be left» and «Top up» when what is
//! left would not pay for this write again (UC-3.2); never a negative «after» (UC-3.3).

use super::prelude::*;
use evolite_api::Duffs;

/// The identity's credits as the snapshot has them (§8d; the architect, 2026-11-25: four states,
/// a failure never a 0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Credits(State);

/// The four states (r2, Must 2): private, so a window cannot make «Known» of a failed read — it
/// has [`Credits::of`], and the token purchase its signed amount ([`Credits::signed`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Not read yet: «checking…».
    Checking,
    /// Read: «now › after».
    Known(Duffs),
    /// The last read failed, an earlier one known: «… (last known) · could not update», no
    /// «after».
    LastKnown(Duffs),
    /// The read failed and nothing is known: «could not read».
    Failed,
}

impl Credits {
    /// The credits of the identity worked as (the one that pays), as the wallet's balances say.
    pub fn of(wallet: &evolite_api::Wallet) -> Self {
        let b = &wallet.balances;
        Self(match (b.credits_read, b.credits_failed) {
            (true, false) => State::Known(b.credits),
            (true, true) => State::LastKnown(b.credits),
            (false, true) => State::Failed,
            (false, false) => State::Checking,
        })
    }

    /// The one exception (BUG-179): a token purchase shows the credits it signed with while it
    /// runs and once done — what the core read for that signature, none yet: «checking…».
    pub fn signed(credits: Option<u64>) -> Self {
        Self(match credits.map(i64::try_from) {
            Some(Ok(c)) => State::Known(c),
            _ => State::Checking,
        })
    }
}

/// Who pays, as the line names it: the identity worked as by its @name, else by its id (as the
/// rest of the app names an identity without one).
pub fn payer_name(wallet: &evolite_api::Wallet) -> String {
    match &wallet.identity {
        Some(i) => match i.name() {
            Some(n) => format!("@{n}"),
            None => crate::text::identity_named(&i.id),
        },
        None => String::new(),
    }
}

/// What the write costs, as the window charges it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Charge {
    /// Still being worked out: «› calculating…».
    Calculating,
    /// An estimate: «after» is «≈ …».
    About(Duffs),
    /// An upper bound (keys with their margin, App Connect's `fee_of_keys`): «after» is «at least …».
    AtMost(Duffs),
}

impl Charge {
    /// An estimate of `duffs` as the window sums it: one too large for the line is
    /// «calculating…», never a negative charge (r2, Should 2: no `as i64` in money).
    pub fn about(duffs: u64) -> Self {
        i64::try_from(duffs).map_or(Self::Calculating, Self::About)
    }

    fn duffs(self) -> Option<Duffs> {
        match self {
            Self::Calculating => None,
            Self::About(d) | Self::AtMost(d) => Some(d),
        }
    }
}

/// What the line says: `(now › after, low)` — `low` when «after» would not pay for the same
/// write again. Pure, for the tests.
pub fn credits_after_words(credits: Credits, charge: Charge) -> (String, bool) {
    // A balance: «•••• DASH» under «Hide amounts» (R-3.2).
    let amount = |d: Duffs| crate::format::balance(d);
    let t = |s: &str| crate::i18n::t(s).into_owned();
    match credits.0 {
        State::Checking => (t(tr!("checking…")), false),
        State::LastKnown(was) => (
            format!(tr!("{} (last known) · could not update"), amount(was)),
            false,
        ),
        State::Failed => (t(tr!("could not read")), false),
        State::Known(now) => match charge {
            Charge::Calculating => (
                format!("{} › {}", amount(now), t(tr!("calculating…"))),
                false,
            ),
            // UC-3.3: not enough already — only «now», the window says why it cannot write.
            c if c.duffs().is_some_and(|d| d > now) => (amount(now), false),
            Charge::About(d) => (
                format!(tr!("{} › ≈ {}"), amount(now), amount(now - d)),
                now - d < d,
            ),
            Charge::AtMost(d) => (
                format!(tr!("{} › at least {}"), amount(now), amount(now - d)),
                now - d < d,
            ),
        },
    }
}

/// The line under «Pays»: «{who} credits» muted on the left, the numbers in mono on the right;
/// when low, amber, «Few credits will be left» and «Top up» under it. Whether «Top up» was
/// clicked: the window closes itself and opens the identity's top-up (`Ctx::top_up`).
pub fn credits_after(
    ui: &mut Ui,
    a: &mut Assets,
    who: &str,
    credits: Credits,
    charge: Charge,
) -> bool {
    let p = a.p;
    let (words, low) = credits_after_words(credits, charge);
    let color = if low { p.accent } else { p.text };
    let label = format!(tr!("{} credits"), who);
    // §8d-2: a long @name and two sums that do not share the row — the sums go under the label
    // (BUG-180, BUG-193), never over it.
    super::kv_row_fit_in(ui, &p, &label, &words, mono(13.0, Weight::Medium), color);
    if !low {
        return false;
    }
    let mut top_up = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(crate::i18n::t(tr!("Few credits will be left")))
                .font(sans(12.0, Weight::Regular))
                .color(p.accent),
        );
        top_up = super::text_action(ui, a, "top-up", tr!("Top up")).clicked();
    });
    // The window stays as it is (its screen keeps it, as «Add a name»'s «Top up ›»); the app
    // opens the top-up of this identity — the one that pays.
    if top_up {
        ui.ctx()
            .data_mut(|d| d.insert_temp(top_up_asked_id(), true));
    }
    top_up
}

/// Where «Top up» of [`credits_after`] is asked for, for `App`.
pub fn top_up_asked_id() -> egui::Id {
    egui::Id::new("credits-after-top-up")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DASH: Duffs = 100_000_000;

    /// R-3.2, R-3.3, UC-3.2, UC-3.3: «after» is «now» less the window's charge, «≈» for an
    /// estimate, «at least» for an upper bound; low when it would not pay for the write again;
    /// none when the credits do not cover it now; the states of §8d.
    #[test]
    fn the_line_follows_the_charge() {
        let (w, low) = credits_after_words(Credits(State::Known(DASH)), Charge::About(DASH / 10));
        assert!(w.contains('›') && w.contains('≈'), "{w}");
        assert!(!low);
        let (w, _) = credits_after_words(Credits(State::Known(DASH)), Charge::AtMost(DASH / 10));
        assert!(w.contains("at least"), "{w}");
        // 1.5 writes' worth: after one, less than one more.
        let (_, low) =
            credits_after_words(Credits(State::Known(3 * DASH / 2)), Charge::About(DASH));
        assert!(low, "few left");
        // Not enough now: only «now».
        let (w, low) = credits_after_words(Credits(State::Known(DASH / 2)), Charge::About(DASH));
        assert!(!w.contains('›') && !low, "{w}");
        assert_eq!(
            credits_after_words(Credits(State::Checking), Charge::About(1)).0,
            "checking…"
        );
        // A failure: the last known amount without «after»; with none, no amount at all.
        let (w, low) =
            credits_after_words(Credits(State::LastKnown(DASH)), Charge::About(DASH / 10));
        assert!(
            w.contains("(last known)") && !w.contains('›') && !low,
            "{w}"
        );
        let (w, _) = credits_after_words(Credits(State::Failed), Charge::About(1));
        assert_eq!(w, "could not read");
        let (w, _) = credits_after_words(Credits(State::Known(DASH)), Charge::Calculating);
        assert!(w.ends_with("calculating…"), "{w}");
    }
}
