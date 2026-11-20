//! FEAT-0004 FR-7 (М4): a spend of more than the wallet's threshold (1 DASH by default) on
//! mainnet asks a tick before its password: «I checked the amount and the recipient». One
//! function says what a spend sends out of the wallet in DASH; the form shows the tick by it and
//! the core refuses without the tick by it, so the two never disagree.

use crate::{Duffs, Fees, Spend};

/// The threshold when the wallet has none saved (or its file cannot be read): 1 DASH. Never «off»
/// by a lost file.
pub const LARGE_SPEND_DEFAULT: Duffs = 100_000_000;

/// What `op` takes out of the wallet in DASH, as the form asked it: the amount sent or locked,
/// with what goes with it. `None`: no amount in DASH — credits for Platform's own operations,
/// contacts, a profile, tokens without a price, and a `Resume*` (its money left earlier).
///
/// Exhaustive, no `_`: a new kind of spend does not build until it is placed here.
pub fn outgoing(op: &Spend, fees: &Fees) -> Option<Duffs> {
    match op {
        Spend::Send { amount, .. } => Some(*amount),
        // What the coins hold over the amount goes with the fee: it leaves the wallet too.
        Spend::SendMixed {
            amount,
            accept_excess,
            ..
        } => Some(amount.saturating_add(*accept_excess)),
        Spend::TopUpIdentity { amount } => Some(*amount),
        Spend::FundAddress { amount } => Some(*amount),
        // The lock: the amount and Platform's fees on top (BUG-93).
        Spend::CreateIdentity { name, top_up } => {
            Some(fees.identity_lock_for(*top_up, name.is_some()))
        }
        Spend::Shield { amount, .. } => Some(*amount),
        Spend::Unshield { amount } => Some(*amount),
        Spend::TopUpFromPool { amount } => Some(*amount),
        // The total agreed, in credits (a thousand to a duff).
        Spend::BuyToken { agreed, .. } => {
            Some(Duffs::try_from(agreed / 1_000).unwrap_or(Duffs::MAX))
        }
        // A claim on a contested name: its fund goes into the vote.
        Spend::AddName { contest_fund, .. } => *contest_fund,
        // FEAT-0038 module V1: the most the registration takes, as its check showed it (with
        // keywords at 0.1 DASH each it may pass the threshold).
        Spend::RegisterContract { fee_max, .. } => Some(*fee_max),
        // FEAT-0038: a series' credits, as its check showed them (architect: the series'
        // estimate; for «Continue», what is left).
        Spend::NftBatch { fee_max, .. }
        | Spend::ContinueSeries { fee_max, .. }
        | Spend::CreateModel { fee_max, .. } => Some(*fee_max),
        Spend::ResumeTopUp
        | Spend::ResumeFundAddress
        | Spend::ResumeIdentity
        | Spend::SetNameLink { .. }
        | Spend::AddContact { .. }
        | Spend::AcceptContact { .. }
        | Spend::SaveProfile { .. }
        | Spend::ApproveApp { .. }
        | Spend::GiftNft { .. }
        | Spend::SendToken { .. }
        | Spend::DisconnectApp { .. }
        | Spend::ClaimToken { .. }
        | Spend::UpdateKeys { .. }
        | Spend::IssuerAction { .. }
        | Spend::ResendRequests { .. } => None,
    }
}

/// Whether changing the threshold from `old` to `new` weakens it — raised, or turned off — and
/// so takes the wallet's password; lowering it or turning it on does not.
pub fn weakens(old: Option<Duffs>, new: Option<Duffs>) -> bool {
    match (old, new) {
        (Some(old), Some(new)) => new > old,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// Whether `op` asks the tick: it sends more than `threshold` (`None`: no threshold — testnet,
/// or turned off). Exactly the threshold does not.
pub fn asks_tick(op: &Spend, fees: &Fees, threshold: Option<Duffs>) -> bool {
    threshold
        .zip(outgoing(op, fees))
        .is_some_and(|(limit, out)| out > limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ShieldSource, Source, tokens};

    fn fees() -> Fees {
        let mut fees = Fees {
            core_payment: 522,
            core_unsignable: 0,
            lock_coins: Vec::new(),
            lock_inputs_max: 0,
            core_coins: Vec::new(),
            credit_transfer: 0,
            credit_transfer_to_address: 0,
            pool_transfer: 0,
            nft_transfer: 0,
            document_write: 0,
            shield: 0,
            identity_create: 0,
            identity_lock_min: 0,
            top_up_lock_min: 50_500,
            address_fund_on_top: 0,
            address_fund_lock_min: 0,
            name_regular: 0,
            name_contested: 0,
            name_contest_fund: 0,
            name_vote_secs: 0,
            name_join_secs: 0,
            add_key: 0,
            key_add_unbound: 0,
            key_add_bound: 0,
            contact_request: 0,
            contact_request_balance: 0,
            profile: 0,
            shield_from_address: 0,
            shield_reserve: 0,
            unshield: 0,
            pool_withdrawal: 0,
            shield_from_core: 0,
            pool_top_up: 0,
            credit_withdrawal: 0,
            credit_withdrawal_reserve: 0,
        };
        fees.identity_create = 1_000_000;
        fees
    }

    /// AC-T4: every kind of spend, the ones in DASH with what they take and the others `None`.
    #[test]
    fn every_spend_says_what_it_takes_in_dash() {
        let f = fees();
        let send = Spend::Send {
            id: 1,
            from: Source::Core,
            to: "yAddr".into(),
            amount: 150_000_000,
            note: None,
            rate: None,
            coins: None,
        };
        let cases: Vec<(Spend, Option<Duffs>)> = vec![
            (send, Some(150_000_000)),
            (
                Spend::SendMixed {
                    id: 1,
                    to: "yAddr".into(),
                    amount: 100_000_000,
                    note: None,
                    rate: None,
                    accept_excess: 2_000,
                },
                Some(100_002_000),
            ),
            (Spend::TopUpIdentity { amount: 7 }, Some(7)),
            (Spend::ResumeTopUp, None),
            (Spend::FundAddress { amount: 8 }, Some(8)),
            (Spend::ResumeFundAddress, None),
            (
                Spend::CreateIdentity {
                    name: None,
                    top_up: 100_000_000,
                },
                Some(101_000_000),
            ),
            (Spend::ResumeIdentity, None),
            (
                Spend::AddName {
                    name: "a".into(),
                    contest_fund: None,
                    link: None,
                },
                None,
            ),
            (
                Spend::AddName {
                    name: "a".into(),
                    contest_fund: Some(20_000_000),
                    link: None,
                },
                Some(20_000_000),
            ),
            (
                Spend::SetNameLink {
                    label: "a".into(),
                    url: None,
                },
                None,
            ),
            (
                Spend::Shield {
                    from: ShieldSource::Core,
                    amount: 9,
                },
                Some(9),
            ),
            (Spend::Unshield { amount: 10 }, Some(10)),
            (Spend::TopUpFromPool { amount: 11 }, Some(11)),
            (Spend::AddContact { name: "a".into() }, None),
            (
                Spend::AcceptContact {
                    identity_id: "a".into(),
                },
                None,
            ),
            (
                Spend::SaveProfile {
                    display: "a".into(),
                    about: String::new(),
                    avatar_url: None,
                },
                None,
            ),
            (Spend::ApproveApp { id: 1, grant: None }, None),
            (
                Spend::GiftNft {
                    id: "a".into(),
                    to: "@a".into(),
                },
                None,
            ),
            (
                Spend::SendToken {
                    token_id: "t".into(),
                    to: "@a".into(),
                    amount: 5,
                },
                None,
            ),
            (Spend::DisconnectApp { id: "a".into() }, None),
            (
                Spend::ClaimToken {
                    token_id: "t".into(),
                    scheduled: false,
                },
                None,
            ),
            (
                Spend::BuyToken {
                    token_id: "t".into(),
                    count: 3,
                    agreed: 150_000_000_000,
                },
                Some(150_000_000),
            ),
            (
                Spend::UpdateKeys {
                    identity: "a".into(),
                    add: Vec::new(),
                    disable: Vec::new(),
                },
                None,
            ),
            (
                Spend::IssuerAction {
                    token_id: "t".into(),
                    op: tokens::IssuerOp::Mint {
                        to: None,
                        amount: 5,
                    },
                },
                None,
            ),
            (
                Spend::ResendRequests {
                    identity: "a".into(),
                    contact: None,
                },
                None,
            ),
            (
                Spend::RegisterContract {
                    identity: "a".into(),
                    draft: crate::contracts::ContractDraft::NftCollection {
                        description: None,
                        keywords: vec!["nft".into()],
                    },
                    fee_max: 29_414_290,
                },
                Some(29_414_290),
            ),
            (
                Spend::NftBatch {
                    target: crate::series::SeriesTarget {
                        identity: "a".into(),
                        contract: "c".into(),
                        model: "m".into(),
                    },
                    kind: crate::series::SeriesKind::Price { count: 3, price: 1 },
                    fee_max: 16_200,
                },
                Some(16_200),
            ),
            (
                Spend::ContinueSeries {
                    id: 1,
                    fee_max: 5_400,
                },
                Some(5_400),
            ),
            (
                Spend::CreateModel {
                    identity: "a".into(),
                    contract: "c".into(),
                    model: crate::series::NewModel {
                        name: "m".into(),
                        description: String::new(),
                        supply: 1,
                        image: Vec::new(),
                    },
                    mint: 1,
                    fee_max: 141_000,
                },
                Some(141_000),
            ),
        ];
        for (op, expected) in cases {
            assert_eq!(outgoing(&op, &f), expected, "{op:?}");
        }
    }

    /// AC-T4: the match has no catch-all, so a new spend cannot slip through as «none».
    #[test]
    fn the_match_names_every_spend() {
        let source = include_str!("confirm.rs");
        let body = &source
            [source.find("pub fn outgoing(").unwrap()..source.find("pub fn asks_tick(").unwrap()];
        assert!(!body.contains("_ =>"), "a catch-all in outgoing");
        let lib = include_str!("lib.rs");
        let spend = &lib[lib.find("pub enum Spend {").unwrap()..];
        let spend = &spend[..spend
            .find("\n}\n")
            .or_else(|| spend.find("\r\n}\r\n"))
            .unwrap()];
        for line in spend.lines() {
            let variant = line
                .strip_prefix("    ")
                .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
                .map(|l| l.split([' ', ',', '{']).next().unwrap_or_default());
            if let Some(variant) = variant {
                assert!(
                    body.contains(&format!("Spend::{variant}")),
                    "{variant} not in outgoing"
                );
            }
        }
    }

    /// AC-T6: raising or turning off takes the password; lowering or turning on does not.
    #[test]
    fn only_a_weaker_threshold_takes_the_password() {
        assert!(weakens(Some(100), Some(101)), "raised");
        assert!(weakens(Some(100), None), "off");
        assert!(!weakens(Some(100), Some(100)), "the same");
        assert!(!weakens(Some(100), Some(50)), "lowered");
        assert!(!weakens(None, Some(1)), "on");
        assert!(!weakens(None, None));
    }

    /// The tick: over the threshold only; exactly it, under it, or no threshold — none.
    #[test]
    fn only_more_than_the_threshold_asks_the_tick() {
        let f = fees();
        let top = |amount| Spend::TopUpIdentity { amount };
        let limit = Some(LARGE_SPEND_DEFAULT);
        assert!(asks_tick(&top(LARGE_SPEND_DEFAULT + 1), &f, limit));
        assert!(
            !asks_tick(&top(LARGE_SPEND_DEFAULT), &f, limit),
            "exactly it"
        );
        assert!(!asks_tick(&top(1), &f, limit));
        assert!(!asks_tick(&top(Duffs::MAX), &f, None), "no threshold");
        assert!(!asks_tick(&Spend::ResumeTopUp, &f, limit), "no amount");
    }
}
