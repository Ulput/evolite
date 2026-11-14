//! Paying from the wallet's Platform addresses (FEAT-0015 stage 1): the recipient says what it
//! is (UC-1) — a Platform address (a transfer, anyone's), a Core address of this wallet (a
//! withdrawal, R-3), one of the wallet's identities by its `@name` (a top-up, R-1а). One payment
//! from the addresses at a time (R-2): each input's nonce goes up with it.

use crate::real::*;
use evolite_api::address_plan::{AddressMove, AddressPlan, duffs_up, move_for, plan_to};
use evolite_chain::chain::addresses::{AddressPaid, AddressPayee};

/// Where a payment from the addresses goes, as read from what was typed.
#[derive(Debug, PartialEq)]
pub(in crate::real) enum AddressTo {
    Platform(String),
    Core(String),
    /// A DPNS label, without `@`.
    Name(String),
}

/// What `to` is for a payment from the addresses; a refusal before anything is signed for what
/// cannot be one: a Platform address of the other network, a Core address of the other network
/// or of another kind (bech32 `td1…`), a name that is not a name.
pub(in crate::real) fn address_to(to: &str) -> Result<AddressTo, CoreError> {
    let to = to.trim();
    match move_for(to) {
        AddressMove::TopUp => Ok(AddressTo::Name(dpns::label(to)?)),
        AddressMove::Transfer => {
            if PlatformAddress::from_bech32m_string(to).is_err() || !platform_address_here(to) {
                return Err(CoreError::BadAddress);
            }
            Ok(AddressTo::Platform(to.to_owned()))
        }
        AddressMove::Withdraw => {
            let address = payee(to)?
                .require_network(crate::network::dash())
                .map_err(|_| CoreError::BadAddress)?;
            Ok(AddressTo::Core(address.to_string()))
        }
    }
}

/// What the fresh plan does against the one the last look showed (the architect, 2026-11-10):
/// a dearer one stops before signing, for the person to look again; a cheaper one goes.
pub(in crate::real) fn fee_still_shown(fresh: &AddressPlan, shown_fee: Option<u64>) -> bool {
    shown_fee.is_none_or(|shown| fresh.fee <= shown)
}

/// The flag of a payment from the addresses on its way, lowered when this is dropped: the task's
/// end, or its panic, so one failure does not refuse every later payment until a restart.
struct Busy(Arc<std::sync::atomic::AtomicBool>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The words for a plan the addresses cannot make.
fn refusal(why: evolite_api::address_plan::AddressRefusal) -> CoreError {
    use evolite_api::address_plan::AddressRefusal;
    match why {
        AddressRefusal::NotEnough { .. } => CoreError::NotEnough,
        AddressRefusal::BelowLeast { .. } => CoreError::BadAmount,
    }
}

/// The credits on the addresses, in whole duffs: what the journal's price line says the core
/// judged a payment by (BUG-141). Rounded down, as a balance that covers is said.
fn held_duffs(credits: &[u64]) -> Duffs {
    let total: u64 = credits.iter().sum();
    (total / evolite_api::address_plan::CREDITS_PER_DUFF) as Duffs
}

impl Real {
    /// `amount` duffs from the Platform addresses to `to`. Answered with [`Event::Sent`] for
    /// payment `payment`, once Platform has taken it.
    pub(in crate::real) fn send_from_addresses(
        &mut self,
        payment: u64,
        id: WalletId,
        to: &str,
        amount: Duffs,
        kept: Kept,
    ) -> Result<(), CoreError> {
        let credits = positive_credits(amount)?;
        // As typed: a transfer never pays from the address it pays to (BUG-141, QA pass 23).
        let typed = to.trim().to_owned();
        let to = address_to(to)?;
        let kind = match &to {
            AddressTo::Platform(_) => AddressMove::Transfer,
            AddressTo::Core(_) => AddressMove::Withdraw,
            AddressTo::Name(_) => AddressMove::TopUp,
        };
        if !self.chain.platform_ready() {
            return Err(CoreError::NotSynced);
        }
        // The plan the last look showed: the one the snapshot's balances give.
        let (shown_names, shown_credits): (Vec<String>, Vec<u64>) =
            self.shown_address_inputs.iter().cloned().unzip();
        let shown_fee = plan_to(kind, credits, false, &shown_credits, &shown_names, &typed)
            .ok()
            .map(|plan| plan.fee);
        let keys = self
            .unlocked
            .as_ref()
            .and_then(|open| open.keys.clone())
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        // One at a time: a second one is refused, not queued (R-2, AC-T4).
        // Not on mainnet until it has been proven live on testnet (review r1, Must 1).
        if crate::network::current() == evolite_api::Network::Mainnet {
            return Err(CoreError::NotAvailable(
                tr!("Payments from a Platform address are not available on mainnet yet").into(),
            ));
        }
        if self.address_busy.swap(true, Ordering::SeqCst) {
            return Err(CoreError::NotAvailable(
                tr!("A payment from the Platform address is on its way: wait for its answer first")
                    .into(),
            ));
        }
        // Lowered on every way out of the task, a panic too (review r1, Should 1).
        let busy = Busy(self.address_busy.clone());
        let chain = self.chain.clone();
        let dirty = self.dirty.clone();
        let journal = self.journal.clone();
        let notes = self.notes.clone();
        let prices = self.prices.clone();
        // In the history as "being sent" until the journal has its line (BUG-12).
        let what = match &to {
            AddressTo::Platform(to) => journal::What::AddressSent { to: to.clone() },
            AddressTo::Core(to) => journal::What::AddressWithdrawal { to: to.clone() },
            AddressTo::Name(label) => journal::What::AddressTopUp {
                identity: format!("@{label}"),
            },
        };
        let sending = self.platform_sending.clone();
        let key = sending.lock().unwrap_or_else(|e| e.into_inner()).start(
            platform_sending::credits_line(&what),
            amount,
            now(),
        );
        self.answer_later("task:send_from_addresses", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let payee = match &to {
                    AddressTo::Platform(to) => AddressPayee::Platform(
                        PlatformAddress::from_bech32m_string(to)
                            .map_err(|_| CoreError::BadAddress)?,
                    ),
                    AddressTo::Core(to) => {
                        let address = payee(to)?
                            .require_network(crate::network::dash())
                            .map_err(|_| CoreError::BadAddress)?;
                        // Only to this wallet's own Core addresses (R-3, analyst-22).
                        if !chain.owns_core_address(&wallet, &address).await {
                            return Err(CoreError::NotAvailable(
                                tr!("From a Platform address to Core: only to this wallet's addresses")
                                    .into(),
                            ));
                        }
                        AddressPayee::Core(address)
                    }
                    AddressTo::Name(label) => match chain.own_identity_named(&wallet, label).await
                    {
                        Some(id) => AddressPayee::Identity(id),
                        // Not one of ours (R-1а): the designer's words are to come.
                        None => {
                            return Err(CoreError::NotAvailable(
                                tr!("From a Platform address, only an identity of this wallet can be topped up")
                                    .into(),
                            ));
                        }
                    },
                };
                // Balances proved now; the plan from them, as the form makes it.
                let balances = chain
                    .address_balances_now(&wallet)
                    .await
                    .map_err(read_failure)?;
                let credits_now: Vec<u64> = balances.iter().map(|(_, c)| *c).collect();
                let names_now: Vec<String> = balances
                    .iter()
                    .map(|(a, _)| a.to_bech32m_string(crate::network::dash()))
                    .collect();
                // The journal's price line says what the core judged by, beside what the form
                // planned with (BUG-141): only the amount.
                prices.held(payment, held_duffs(&credits_now));
                let plan = plan_to(kind, credits, false, &credits_now, &names_now, &typed)
                    .map_err(refusal)?;
                if !fee_still_shown(&plan, shown_fee) {
                    return Err(CoreError::NotAvailable(
                        tr!("The fee has grown since the last look: check the payment again")
                            .into(),
                    ));
                }
                let AddressPaid {
                    before,
                    after,
                    document,
                } = chain
                    .address_pay(&wallet, &keys, payee, &plan, &balances)
                    .await
                    .map_err(dashpay_failure)?;
                // What left the addresses; the plan's fee when the read back did not move yet.
                let spent = duffs_up(before.saturating_sub(after).max(plan.spent()));
                let moved = (plan.receives / evolite_api::address_plan::CREDITS_PER_DUFF) as Duffs;
                let line = journal.add(journal::Entry {
                    at: now(),
                    what,
                    spent,
                    moved,
                    key: document,
                    ..Default::default()
                });
                if let Some(line) = line.filter(|_| !kept.is_empty()) {
                    let key = format!("platform:{line:016x}");
                    keep_note(notes, move |n| kept.into_notes(n, &key)).await;
                }
                Ok::<Duffs, CoreError>(spent - moved)
            }
            .await
            .map(|fee| SentPayment {
                txid: String::new(),
                amount,
                fee,
                relayed: true,
            });
            drop(busy);
            sending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .answered(key, result.is_ok(), now());
            dirty.store(true, Ordering::Relaxed);
            Event::Sent {
                id: payment,
                result,
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-T1: what the recipient makes of a payment from the addresses, refused before signing
    /// when it cannot be one. (Whose Core address and whose name: checked on the wallet.)
    #[test]
    fn the_recipient_chooses_the_operation() {
        let platform = "tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq";
        if PlatformAddress::from_bech32m_string(platform).is_ok() {
            assert_eq!(
                address_to(platform),
                Ok(AddressTo::Platform(platform.into()))
            );
        }
        assert_eq!(
            address_to("@Kestrel7"),
            Ok(AddressTo::Name("kestrel7".into()))
        );
        assert!(matches!(
            address_to("yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f"),
            Ok(AddressTo::Core(_)) | Err(CoreError::BadAddress)
        ));
        // bech32 Core, mainnet Platform, mainnet Core, garbage: refused before signing.
        for wrong in [
            "td1qxyzxyzxyzxyzxyzxyzxyzxyz",
            "dash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq",
            "XkGQ7h8eKz8pZdNNfz3JE5ZPHJ7Gm7e2aQ",
            "nonsense",
        ] {
            assert!(address_to(wrong).is_err(), "{wrong}");
        }
    }

    /// Review r1, Should 1: the flag of a payment on its way is lowered on every way out of its
    /// task, a panic too; otherwise every later payment from the addresses is refused.
    #[test]
    fn the_busy_flag_is_lowered_even_by_a_panic() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let busy = Busy(flag.clone());
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _busy = busy;
            panic!("the task failed");
        }));
        assert!(failed.is_err());
        assert!(!flag.load(Ordering::SeqCst), "lowered by the panic");
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        drop(Busy(flag.clone()));
        assert!(!flag.load(Ordering::SeqCst), "lowered at the end");
    }

    /// The architect, 2026-11-10: a fresh plan dearer than the shown one stops; cheaper goes.
    #[test]
    fn a_dearer_fresh_plan_stops_before_signing() {
        let plan = |fee| AddressPlan {
            inputs: vec![(0, 1_000_000)],
            fee,
            fee_payer: Some(0),
            receives: 1_000_000,
        };
        assert!(fee_still_shown(&plan(6_500_000), Some(6_500_000)));
        assert!(fee_still_shown(&plan(6_500_000), Some(7_000_000)));
        assert!(!fee_still_shown(&plan(7_000_000), Some(6_500_000)));
        // Nothing shown could be planned (a stale cache): the fresh plan decides.
        assert!(fee_still_shown(&plan(7_000_000), None));
    }
}

#[cfg(test)]
mod held_tests {
    use super::held_duffs;

    /// BUG-141: the balance in the journal is whole duffs of all the addresses' credits,
    /// rounded down.
    #[test]
    fn the_held_balance_is_whole_duffs_of_all_the_addresses() {
        assert_eq!(held_duffs(&[]), 0);
        assert_eq!(held_duffs(&[2_875_110_000]), 2_875_110);
        assert_eq!(held_duffs(&[1_999, 1_000]), 2);
    }
}
