//! Which balances an operation changes and which way it moved money for the whole wallet
//! ([`Activity::layers`], [`Activity::flow`], FEAT-0009 R-1 and R-2): the screens cannot tell
//! a shield's source, a lock's target or whose address a withdrawal paid; the core can.

use evolite_api::layer::{CORE, IDENTITY, PLATFORM_ADDRESS, POOL};
use evolite_api::{Activity, ActivityKind, Chain, Flow, Wallet};

/// The label a shield from an identity's credits is kept with (the journal's `Shield::from`).
pub const FROM_CREDITS: &str = tr!("identity credits");
/// The label a shield from the Platform address is kept with.
pub const FROM_PLATFORM_ADDRESS: &str = tr!("a Platform address");
/// The label of a shield from Core (an asset lock made for the pool).
pub const FROM_CORE: &str = "Core";

/// What the wallet holds that an operation may pay: its own Core addresses, identities (as
/// `@name` and as base58 id) and Platform address.
#[derive(Default)]
pub struct Own<'a> {
    pub core_addresses: Vec<&'a str>,
    pub identities: Vec<&'a str>,
    pub platform_address: Option<&'a str>,
}

impl Own<'_> {
    fn core(&self, address: &str) -> bool {
        !address.is_empty() && self.core_addresses.contains(&address)
    }

    fn identity(&self, who: &str) -> bool {
        // A name as the history writes it, `@name`, or as the identity has it.
        !who.is_empty()
            && (self.identities.contains(&who)
                || who
                    .strip_prefix('@')
                    .is_some_and(|name| self.identities.contains(&name)))
    }

    fn platform_address(&self, to: &str) -> bool {
        !to.is_empty() && self.platform_address == Some(to)
    }
}

/// The layers and flow of a line by its kind and chain alone: what every line gets where it
/// is made. [`settle`] then adds what only the wallet's own addresses and identities say.
pub fn of(kind: &ActivityKind, chain: Chain) -> (u8, Flow) {
    use ActivityKind::*;
    match kind {
        Received { .. } => (CORE, Flow::In),
        Sent { .. } => (CORE, Flow::Out),
        // The pool's own transfer to a private address of ours is on Platform.
        SelfTransfer { .. } if chain == Chain::Platform => (POOL, Flow::Between),
        SelfTransfer { .. } => (CORE, Flow::Between),
        PlatformReceived => (PLATFORM_ADDRESS, Flow::In),
        // An asset lock: Core to the identity it registers or tops up, or to the Platform
        // address (no identity and no registration).
        TopUp {
            identity, creates, ..
        } if identity.is_empty() && !creates => (CORE | PLATFORM_ADDRESS, Flow::Between),
        TopUp { .. } => (CORE | IDENTITY, Flow::Between),
        Shielded { from } => {
            let source = match from.as_deref() {
                Some(FROM_CORE) => CORE,
                Some(FROM_CREDITS) => IDENTITY,
                Some(FROM_PLATFORM_ADDRESS) => PLATFORM_ADDRESS,
                // A shield the pool's scan found, its source not known.
                _ => 0,
            };
            (source | POOL, Flow::Between)
        }
        PoolReceived => (POOL, Flow::In),
        PoolSent => (POOL, Flow::Out),
        // Paid in credits by the identity; the payout is a line of its own (Core, received).
        Withdrawal { .. } | CreditTransfer { .. } => (IDENTITY, Flow::Out),
        AppSignIn { .. } | ContactRequest { .. } | ProfileSaved | NameRegistered { .. } => {
            (IDENTITY, Flow::Out)
        }
        // Costs nothing: someone asked us.
        ContactRequestIn { .. } => (IDENTITY, Flow::In),
    }
}

/// A line's layers and flow once the history is put together: a lock's target and a Core
/// lock made for the pool are known only then (taken again from the kind), a withdrawal to an
/// address of ours and credits to an identity of ours are moves between own balances, and
/// credits to our Platform address change that balance too. Anything else keeps what it was
/// made with (a shield the pool's scan found knows its source only there).
pub fn settle(act: &mut Activity, own: &Own) {
    match &act.kind {
        ActivityKind::TopUp { .. } | ActivityKind::Shielded { from: Some(_) } => {
            (act.layers, act.flow) = of(&act.kind, act.chain);
        }
        ActivityKind::Withdrawal { to, .. } => {
            act.flow = if own.core(to) {
                Flow::Between
            } else {
                Flow::Out
            };
        }
        ActivityKind::CreditTransfer { to } if own.identity(to) => {
            (act.layers, act.flow) = (IDENTITY, Flow::Between);
        }
        ActivityKind::CreditTransfer { to } if own.platform_address(to) => {
            (act.layers, act.flow) = (IDENTITY | PLATFORM_ADDRESS, Flow::Between);
        }
        _ => {}
    }
}

/// [`settle`]s every line of the snapshot's history and of its pool with what is the
/// wallet's own: `own_core`, its Core addresses (the snapshot has only the one to receive
/// on), and from the snapshot itself its identities and Platform address.
pub fn settle_wallet(wallet: &mut Wallet, own_core: &[String]) {
    let Wallet {
        activity,
        pool,
        identities,
        platform_address,
        ..
    } = wallet;
    let own = Own {
        core_addresses: own_core.iter().map(String::as_str).collect(),
        identities: identities
            .iter()
            .flat_map(|i| [Some(i.id.as_str()), i.name.as_deref()])
            .flatten()
            .collect(),
        platform_address: Some(platform_address.as_str()),
    };
    for act in activity.iter_mut().chain(pool.operations.iter_mut()) {
        settle(act, &own);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{Credits, IdentityChoice, Proof, RequestState};

    fn line(kind: ActivityKind, chain: Chain) -> Activity {
        let (layers, flow) = of(&kind, chain);
        Activity {
            id: 1,
            kind,
            chain,
            at: 0,
            amount: 0,
            proof: Proof::Proved,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers,
            flow,
        }
    }

    fn lock(identity: &str, creates: bool) -> ActivityKind {
        ActivityKind::TopUp {
            identity: identity.into(),
            creates,
            credits: Credits::Added,
        }
    }

    fn shield(from: &str) -> ActivityKind {
        ActivityKind::Shielded {
            from: Some(from.into()),
        }
    }

    /// AC-T1: every kind of line and the balances it changes (R-1).
    #[test]
    fn each_line_is_in_every_layer_whose_money_it_moves() {
        use ActivityKind::*;
        let table = [
            (
                lock("@kestrel7", false),
                Chain::CoreToPlatform,
                CORE | IDENTITY,
            ),
            (lock("", true), Chain::CoreToPlatform, CORE | IDENTITY),
            (
                lock("", false),
                Chain::CoreToPlatform,
                CORE | PLATFORM_ADDRESS,
            ),
            (shield(FROM_CREDITS), Chain::Platform, IDENTITY | POOL),
            (shield(FROM_CORE), Chain::Platform, CORE | POOL),
            (
                shield(FROM_PLATFORM_ADDRESS),
                Chain::Platform,
                PLATFORM_ADDRESS | POOL,
            ),
            (Shielded { from: None }, Chain::Platform, POOL),
            (
                Withdrawal {
                    to: "yOther".into(),
                    status: None,
                },
                Chain::Platform,
                IDENTITY,
            ),
            // The withdrawal's payout.
            (Received { from: None }, Chain::Core, CORE),
            (
                CreditTransfer { to: "@bob".into() },
                Chain::Platform,
                IDENTITY,
            ),
            (PoolReceived, Chain::Platform, POOL),
            (PoolSent, Chain::Platform, POOL),
            (SelfTransfer { moved: 0 }, Chain::Platform, POOL),
            (
                NameRegistered {
                    label: "kestrel".into(),
                },
                Chain::Platform,
                IDENTITY,
            ),
            (
                ContactRequest {
                    to: "@bob".into(),
                    accepting: false,
                    ours: None,
                },
                Chain::Platform,
                IDENTITY,
            ),
            (
                ContactRequestIn {
                    from: "@bob".into(),
                    ours: None,
                    state: RequestState::Waiting,
                },
                Chain::Platform,
                IDENTITY,
            ),
            (ProfileSaved, Chain::Platform, IDENTITY),
            (
                AppSignIn {
                    app: "Yappr".into(),
                },
                Chain::Platform,
                IDENTITY,
            ),
            (Sent { to: "yX".into() }, Chain::Core, CORE),
            (SelfTransfer { moved: 5 }, Chain::Core, CORE),
            (PlatformReceived, Chain::Platform, PLATFORM_ADDRESS),
        ];
        let own = Own::default();
        for (kind, chain, layers) in table {
            let mut act = line(kind.clone(), chain);
            settle(&mut act, &own);
            assert_eq!(act.layers, layers, "{kind:?}");
            assert_ne!(act.layers, 0, "{kind:?}");
        }
    }

    /// AC-T2: the direction for the whole wallet (R-2) — every move between own balances, and
    /// a withdrawal or credits to someone else's as out.
    #[test]
    fn only_the_fee_leaving_is_between_your_own() {
        use ActivityKind::*;
        let own = Own {
            core_addresses: vec!["yMine"],
            identities: vec!["@kestrel7", "4EfA9Jrvv3nnCFdSf7fad59851iiTRZ6Wcu6YVJ4iSeF"],
            platform_address: Some("tdash1mine"),
        };
        let flow = |kind: ActivityKind, chain: Chain| {
            let mut act = line(kind, chain);
            settle(&mut act, &own);
            (act.flow, act.layers)
        };
        let between = [
            (SelfTransfer { moved: 1 }, Chain::Core),
            (lock("@kestrel7", false), Chain::CoreToPlatform),
            (lock("", false), Chain::CoreToPlatform),
            (shield(FROM_CORE), Chain::Platform),
            (shield(FROM_CREDITS), Chain::Platform),
            (
                Withdrawal {
                    to: "yMine".into(),
                    status: None,
                },
                Chain::Platform,
            ),
            (
                CreditTransfer {
                    to: "@kestrel7".into(),
                },
                Chain::Platform,
            ),
            (
                CreditTransfer {
                    to: "4EfA9Jrvv3nnCFdSf7fad59851iiTRZ6Wcu6YVJ4iSeF".into(),
                },
                Chain::Platform,
            ),
        ];
        for (kind, chain) in between {
            assert_eq!(flow(kind.clone(), chain).0, Flow::Between, "{kind:?}");
        }
        assert_eq!(
            flow(
                CreditTransfer {
                    to: "tdash1mine".into()
                },
                Chain::Platform
            ),
            (Flow::Between, IDENTITY | PLATFORM_ADDRESS)
        );
        let out = [
            Sent { to: "yX".into() },
            Withdrawal {
                to: "yOther".into(),
                status: None,
            },
            CreditTransfer { to: "@bob".into() },
            PoolSent,
        ];
        for kind in out {
            assert_eq!(flow(kind.clone(), Chain::Core).0, Flow::Out, "{kind:?}");
        }
        for kind in [Received { from: None }, PlatformReceived, PoolReceived] {
            assert_eq!(flow(kind.clone(), Chain::Core).0, Flow::In, "{kind:?}");
        }
    }

    /// Review r1 must 1: the snapshot's pass over its history and its pool. Lines are made
    /// before the wallet knows what they are for (`Real::wallet`): a lock's identity comes from
    /// `lock_line`, a lock made for the pool turns into a shield, and the pool's own lines are
    /// settled too.
    #[test]
    fn the_snapshot_settles_its_history_and_its_pool_with_what_is_its_own() {
        use ActivityKind::*;
        const ID: &str = "4EfA9Jrvv3nnCFdSf7fad59851iiTRZ6Wcu6YVJ4iSeF";
        let mut wallet = crate::mock::example_wallet(0);
        wallet.identities = vec![IdentityChoice {
            id: ID.into(),
            name: Some("kestrel7".into()),
            slot: 0,
        }];
        wallet.platform_address = "tdash1mine".into();
        // Made as a lock to the Platform address; its identity is filled in afterwards.
        let mut top_up = line(lock("", false), Chain::CoreToPlatform);
        top_up.kind = lock("@kestrel7", false);
        // Made as a lock; then known to be the pool's.
        let mut from_core = line(lock("", false), Chain::CoreToPlatform);
        from_core.kind = shield(FROM_CORE);
        let withdrawal = |to: &str| {
            line(
                Withdrawal {
                    to: to.into(),
                    status: None,
                },
                Chain::Platform,
            )
        };
        let credits = |to: &str| line(CreditTransfer { to: to.into() }, Chain::Platform);
        wallet.activity = vec![
            top_up,
            from_core,
            withdrawal("yMine"),
            withdrawal("yOther"),
            credits("@kestrel7"),
            credits(ID),
            credits("tdash1mine"),
            credits("@bob"),
        ];
        // A shield the pool's scan found, its source learnt later.
        let mut scanned = line(Shielded { from: None }, Chain::Platform);
        scanned.kind = shield(FROM_CREDITS);
        let mut to_mine = credits("@kestrel7");
        to_mine.flow = Flow::Out;
        wallet.pool.operations = vec![scanned, to_mine];

        settle_wallet(&mut wallet, &["yMine".to_string()]);

        let got: Vec<_> = wallet.activity.iter().map(|a| (a.layers, a.flow)).collect();
        assert_eq!(
            got,
            [
                (CORE | IDENTITY, Flow::Between),
                (CORE | POOL, Flow::Between),
                (IDENTITY, Flow::Between),
                (IDENTITY, Flow::Out),
                (IDENTITY, Flow::Between),
                (IDENTITY, Flow::Between),
                (IDENTITY | PLATFORM_ADDRESS, Flow::Between),
                (IDENTITY, Flow::Out),
            ]
        );
        let pool: Vec<_> = wallet
            .pool
            .operations
            .iter()
            .map(|a| (a.layers, a.flow))
            .collect();
        assert_eq!(
            pool,
            [(IDENTITY | POOL, Flow::Between), (IDENTITY, Flow::Between)]
        );
    }
}
