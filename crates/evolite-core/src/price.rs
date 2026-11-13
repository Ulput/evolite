//! What a spend was said to cost, for the journal's price record (TECH-price-check): the same
//! `Fees` the forms read, from the snapshot the person saw. Pure, so a test can hold each kind
//! against what its form shows. Only the price is here: no address, name or amount leaves.

use evolite_api::{AppRequestKind, CoreError, Duffs, Fees, ShieldSource, Source, Spend, Wallet};

/// What the person saw, from the last snapshot: the prices, and the balances they are paid from.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub fees: Fees,
    pub core: Duffs,
    pub credits: Duffs,
    pub platform_address: Duffs,
    pub pool: Duffs,
    /// The fully mixed coins a payment from «Mixed» may spend (FEAT-0030).
    pub mixed: Duffs,
    /// The credits on each Platform address, in their own order (FEAT-0015).
    pub platform_inputs: Vec<u64>,
    /// The identity shown has no name yet: its first costs more (`Fees::name_price`).
    pub first_name: bool,
}

impl Seen {
    pub fn of(wallet: &Wallet) -> Self {
        Self {
            fees: wallet.fees.clone(),
            core: wallet.balances.core,
            credits: wallet.balances.credits,
            platform_address: wallet.balances.platform_address,
            pool: wallet.balances.shielded,
            mixed: wallet.balances.mixed.map_or(0, |m| m.spendable),
            platform_inputs: wallet.platform_inputs.clone(),
            first_name: wallet
                .identity
                .as_ref()
                .is_none_or(|identity| identity.names.is_empty()),
        }
    }
}

/// A spend's price as it was promised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Declared {
    pub what: &'static str,
    pub declared: Duffs,
    /// The balance it is paid from covered the price (and what it moves) when asked.
    pub enough: bool,
}

/// Credits to `to` go to a Core address: a withdrawal (as the Send form tells them apart).
fn to_core_address(to: &str) -> bool {
    let to = to.trim();
    !(to.is_empty() || to.starts_with('@') || to.to_ascii_lowercase().starts_with("tdash1"))
}

/// What a payment from the pool to `to` costs, as the Send form says it.
fn pool_price(fees: &Fees, to: &str) -> Duffs {
    let to = to.trim();
    if to.is_empty() || to.starts_with('@') {
        return fees.pool_transfer;
    }
    if to.to_ascii_lowercase().starts_with("tdash1") {
        // A private address is much longer than a Platform address.
        return if to.len() > 64 {
            fees.pool_transfer
        } else {
            fees.unshield
        };
    }
    fees.pool_withdrawal
}

/// What `op` was said to cost. `app_cost`: the cost its request showed, for [`Spend::ApproveApp`].
/// Without a catch-all: a new kind of spend must say its price.
pub fn declared(op: &Spend, seen: &Seen, app_cost: Duffs) -> Declared {
    let fees = &seen.fees;
    let covers = |have: Duffs, moved: Duffs, price: Duffs| moved.saturating_add(price) <= have;
    let of = |what, declared, enough| Declared {
        what,
        declared,
        enough,
    };
    match op {
        Spend::Send {
            from: Source::Core,
            amount,
            ..
        } => {
            let price = fees.core_payment_for(*amount);
            of("send_core", price, covers(seen.core, *amount, price))
        }
        // FEAT-0030: the usual fee plus the excess the person agreed to burn, against the fully
        // mixed coins (the core checks the coins themselves too: `NotEnough`).
        // `Spend::Send` from «Mixed» is refused (it goes as `SendMixed`): priced the same.
        Spend::Send {
            from: Source::Mixed,
            amount,
            ..
        } => {
            let price = fees.core_payment_for(*amount);
            of("send_mixed", price, covers(seen.mixed, *amount, price))
        }
        Spend::SendMixed {
            amount,
            accept_excess,
            ..
        } => {
            let price = fees
                .core_payment_for(*amount)
                .saturating_add(*accept_excess);
            of("send_mixed", price, covers(seen.mixed, *amount, price))
        }
        Spend::Send {
            from: Source::Credits,
            to,
            amount,
            ..
        } => {
            let (what, price) = if to_core_address(to) {
                ("withdraw_credits", fees.credit_withdrawal)
            } else if to.trim().to_ascii_lowercase().starts_with("tdash1") {
                ("send_credits", fees.credit_transfer_to_address)
            } else {
                ("send_credits", fees.credit_transfer)
            };
            of(what, price, covers(seen.credits, *amount, price))
        }
        Spend::Send {
            from: Source::Pool,
            to,
            amount,
            ..
        } => {
            let price = pool_price(fees, to);
            of("send_pool", price, covers(seen.pool, *amount, price))
        }
        // The plan the form shows (FEAT-0015 R-7): its fee, and whether the addresses carry it.
        Spend::Send {
            from: Source::PlatformAddress,
            to,
            amount,
            ..
        } => {
            use evolite_api::address_plan::{AddressMove, address_plan, duffs_up, move_for};
            let kind = move_for(to);
            let credits = ((*amount).max(0) as u64)
                .saturating_mul(evolite_api::address_plan::CREDITS_PER_DUFF);
            let (price, enough) = match address_plan(kind, credits, false, &seen.platform_inputs) {
                Ok(plan) => (duffs_up(plan.fee), true),
                Err(_) => (duffs_up(kind.fee(1)), false),
            };
            let what = match kind {
                AddressMove::Transfer => "address_transfer",
                AddressMove::Withdraw => "address_withdraw",
                AddressMove::TopUp => "address_top_up",
            };
            of(what, price, enough)
        }
        Spend::TopUpIdentity { amount } => {
            let price = fees.top_up_on_top().saturating_add(fees.core_payment);
            of("top_up", price, covers(seen.core, *amount, price))
        }
        Spend::CreateIdentity { name, top_up } => {
            let named = name.is_some();
            let core = fees.core_payment_for(fees.identity_lock_for(*top_up, named));
            let price = fees.identity_fees_on_top(named).saturating_add(core);
            of("create_identity", price, covers(seen.core, *top_up, price))
        }
        // The lock is paid for already: nothing more was promised.
        Spend::ResumeTopUp => of("resume_top_up", 0, true),
        Spend::ResumeIdentity => of("resume_identity", 0, true),
        Spend::AddName { .. } => {
            let price = fees.name_price(seen.first_name);
            of("add_name", price, covers(seen.credits, 0, price))
        }
        Spend::Shield {
            from: ShieldSource::Credits,
            amount,
        } => of(
            "shield_credits",
            fees.shield,
            covers(seen.credits, *amount, fees.shield),
        ),
        Spend::Shield {
            from: ShieldSource::PlatformAddress,
            amount,
        } => of(
            "shield_address",
            fees.shield_from_address,
            covers(seen.platform_address, *amount, fees.shield_from_address),
        ),
        // Platform's fee comes out of the lock; only the Core fee is on top of the amount.
        Spend::Shield {
            from: ShieldSource::Core,
            amount,
        } => of(
            "shield_core",
            fees.shield_from_core.saturating_add(fees.core_payment),
            covers(seen.core, *amount, fees.core_payment),
        ),
        Spend::Unshield { amount } => of(
            "unshield",
            fees.unshield,
            covers(seen.pool, *amount, fees.unshield),
        ),
        Spend::TopUpFromPool { amount } => of(
            "top_up_from_pool",
            fees.pool_top_up,
            covers(seen.pool, *amount, fees.pool_top_up),
        ),
        Spend::AddContact { .. } => of(
            "add_contact",
            fees.contact_request,
            covers(seen.credits, 0, fees.contact_request),
        ),
        Spend::AcceptContact { .. } => of(
            "accept_contact",
            fees.contact_request,
            covers(seen.credits, 0, fees.contact_request),
        ),
        Spend::SaveProfile { .. } => of(
            "save_profile",
            fees.profile,
            covers(seen.credits, 0, fees.profile),
        ),
        Spend::ApproveApp { .. } => of("approve_app", app_cost, covers(seen.credits, 0, app_cost)),
        Spend::GiftNft { .. } => of(
            "gift_nft",
            fees.nft_transfer,
            covers(seen.credits, 0, fees.nft_transfer),
        ),
        // The token itself is not credits: only the fee comes from them. A token transfer is a
        // batch transition as a document write is. Until AC-T9 measures its own fee, twice a
        // document write: the estimate must not be below what is taken (part 3 review r1, answer 3).
        Spend::SendToken { .. } => of(
            "send_token",
            fees.document_write.saturating_mul(2),
            covers(seen.credits, 0, fees.document_write.saturating_mul(2)),
        ),
        // One IdentityUpdate that disables the session keys.
        Spend::DisconnectApp { .. } => of(
            "disconnect_app",
            fees.add_key,
            covers(seen.credits, 0, fees.add_key),
        ),
    }
}

/// The cost an app's request showed.
pub fn app_cost(kind: &AppRequestKind) -> Duffs {
    match kind {
        AppRequestKind::Connect2 { cost, .. } => *cost,
    }
}

/// Why a spend failed, as a code. The network's text is only looked at, never kept.
pub fn reason(error: &CoreError) -> &'static str {
    match error {
        CoreError::NotEnough => "not_enough",
        CoreError::NotSynced => "network",
        CoreError::Failed(text) if about_the_network(text) => "network",
        CoreError::Failed(_) => "refused",
        CoreError::NotAvailable(_) => "not_available",
        CoreError::WrongPassword => "wrong_password",
        CoreError::BadAddress
        | CoreError::BadAmount
        | CoreError::BadName
        | CoreError::NameNotFound
        | CoreError::BadLink
        | CoreError::AppLink(_)
        | CoreError::BadPhrase => "bad_input",
        CoreError::TooManyTries { .. }
        | CoreError::PicturesOff(_)
        | CoreError::MixedExcess { .. } => "refused",
    }
}

fn about_the_network(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "timeout",
        "timed out",
        "connection",
        "unavailable",
        "no available",
        "transport",
        "dapi",
    ]
    .iter()
    .any(|word| text.contains(word))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use evolite_api::NoteText;

    /// Every field differs, so a price read from the wrong one is seen.
    pub(crate) fn fees() -> Fees {
        Fees {
            core_payment: 11,
            core_unsignable: 0,
            core_coins: Vec::new(),
            credit_transfer: 12,
            credit_transfer_to_address: 33,
            pool_transfer: 13,
            nft_transfer: 14,
            document_write: 15,
            shield: 16,
            identity_create: 17,
            identity_lock_min: 18,
            top_up_lock_min: 19,
            name_regular: 20,
            name_contested: 21,
            add_key: 22,
            contact_request: 23,
            contact_request_balance: 34,
            profile: 24,
            shield_from_address: 25,
            shield_reserve: 26,
            unshield: 27,
            pool_withdrawal: 28,
            shield_from_core: 29,
            pool_top_up: 30,
            credit_withdrawal: 31,
            credit_withdrawal_reserve: 32,
        }
    }

    pub(crate) fn seen(balance: Duffs) -> Seen {
        Seen {
            fees: fees(),
            core: balance,
            credits: balance,
            platform_address: balance,
            pool: balance,
            mixed: balance,
            platform_inputs: vec![balance.max(0) as u64 * 1_000],
            first_name: false,
        }
    }

    /// One of every spend, with an amount of 100 where there is one. `declared` has no catch-all,
    /// so a new variant does not build until it says its price; this list is checked against it.
    pub(crate) fn every_spend() -> Vec<Spend> {
        let send = |from, to: &str| Spend::Send {
            coins: None,
            id: 1,
            from,
            to: to.into(),
            amount: 100,
            note: None::<NoteText>,
            rate: None,
        };
        vec![
            send(Source::Core, "yAddress"),
            send(Source::Credits, "@bob"),
            send(Source::Credits, "yAddress"),
            send(Source::Pool, "@bob"),
            Spend::SendMixed {
                id: 1,
                to: "yAddress".into(),
                amount: 100,
                note: None,
                rate: None,
                accept_excess: 0,
            },
            Spend::TopUpIdentity { amount: 100 },
            Spend::ResumeTopUp,
            Spend::CreateIdentity {
                name: Some("bob".into()),
                top_up: 100,
            },
            Spend::ResumeIdentity,
            Spend::AddName { name: "bob".into() },
            Spend::Shield {
                from: ShieldSource::Credits,
                amount: 100,
            },
            Spend::Shield {
                from: ShieldSource::PlatformAddress,
                amount: 100,
            },
            Spend::Shield {
                from: ShieldSource::Core,
                amount: 100,
            },
            Spend::Unshield { amount: 100 },
            Spend::TopUpFromPool { amount: 100 },
            Spend::AddContact { name: "bob".into() },
            Spend::AcceptContact {
                identity_id: "id".into(),
            },
            Spend::SaveProfile {
                display: "Bob".into(),
                about: String::new(),
                avatar_url: None,
            },
            Spend::ApproveApp { id: 7, grant: None },
            Spend::GiftNft {
                id: "nft".into(),
                to: "@bob".into(),
            },
            Spend::SendToken {
                token_id: "token".into(),
                to: "@bob".into(),
                amount: 100,
            },
            Spend::DisconnectApp { id: "app".into() },
        ]
    }

    #[test]
    fn every_kind_of_spend_has_its_own_name() {
        let names: Vec<&str> = every_spend()
            .iter()
            .map(|op| declared(op, &seen(1_000), 5).what)
            .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{names:?}");
    }

    #[test]
    fn enough_is_the_price_and_what_is_moved_against_the_balance() {
        for op in every_spend() {
            let d = declared(&op, &seen(0), 5);
            let free = d.declared == 0;
            assert_eq!(d.enough, free, "{}: nothing to pay from", d.what);
            assert!(declared(&op, &seen(1_000_000_000), 5).enough, "{}", d.what);
        }
        // Exactly the amount and the price: enough; a duff less: not.
        let op = Spend::Unshield { amount: 100 };
        assert!(declared(&op, &seen(127), 0).enough);
        assert!(!declared(&op, &seen(126), 0).enough);
    }

    #[test]
    fn an_apps_price_is_the_one_its_request_showed() {
        let d = declared(&Spend::ApproveApp { id: 7, grant: None }, &seen(100), 42);
        assert_eq!((d.what, d.declared), ("approve_app", 42));
    }

    /// FEAT-0023: disconnecting an app is one IdentityUpdate, priced and named as such.
    #[test]
    fn a_disconnect_costs_one_identity_update() {
        let op = Spend::DisconnectApp { id: "app".into() };
        let d = declared(&op, &seen(100), 0);
        assert_eq!((d.what, d.declared), ("disconnect_app", fees().add_key));
    }

    /// The code says what happened; the network's words (with an address, a name, a host) stay out.
    #[test]
    fn a_reason_is_a_code_and_keeps_no_text() {
        let address = "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ";
        assert_eq!(reason(&CoreError::NotEnough), "not_enough");
        assert_eq!(reason(&CoreError::NotSynced), "network");
        for (text, code) in [
            (format!("timed out at 10.0.0.1 for {address}"), "network"),
            (format!("DAPI gave up on {address}"), "network"),
            (format!("document {address} is invalid"), "refused"),
        ] {
            let got = reason(&CoreError::Failed(text));
            assert_eq!(got, code);
            assert!(!got.contains(address));
        }
        assert_eq!(reason(&CoreError::BadAddress), "bad_input");
    }
}
