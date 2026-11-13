//! Paying from the wallet's Platform addresses (FEAT-0015): a transfer to a Platform address, a
//! withdrawal to Core, a top-up of one of the wallet's identities. The inputs are the ones
//! `evolite_api::address_plan` chose from balances proved just before (the architect,
//! 2026-11-10); nothing is left to platform-wallet's own choice, and no change output is made:
//! what an address does not give stays on it.

use super::*;
use dpp::address_funds::AddressFundsFeeStrategyStep;
use dpp::identity::core_script::CoreScript;
use evolite_api::address_plan::AddressPlan;
use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use platform_wallet::wallet::platform_addresses::InputSelection;

/// The Platform payment account evolite spends from and receives on (DIP-17 account 0).
const ACCOUNT: u32 = 0;
/// Core's fee per byte for a withdrawal's payout: the network's least.
const CORE_FEE_PER_BYTE: u32 = 1;

/// Where the money from the addresses goes.
pub enum AddressPayee {
    Platform(PlatformAddress),
    /// A Core address of this wallet (the core checks it is one, R-3).
    Core(Address),
    /// One of the wallet's identities.
    Identity(Identifier),
}

/// What the network did with a payment from the addresses.
pub struct AddressPaid {
    /// What the paying addresses held before and hold now, in credits: the difference is what
    /// left them, fee included.
    pub before: u64,
    pub after: u64,
    /// A withdrawal's document on Platform (`wd:<id>`), from the transition itself.
    pub document: Option<String>,
}

/// A signer of the wallet's Platform addresses: every address of the payment account that
/// platform-wallet knows, with its private key derived from `keys` (dropped with the signer).
pub(crate) async fn address_signer(
    wallet: &PlatformWallet,
    keys: &key_wallet::Wallet,
) -> Result<SimpleSigner, String> {
    let state = wallet.state().await;
    let account = state
        .core_wallet
        .platform_payment_managed_account_at_index(ACCOUNT)
        .ok_or(tr!("The wallet has no Platform addresses"))?;
    let mut signer = SimpleSigner::default();
    for info in account.addresses.addresses.values() {
        let Ok(address) = key_wallet::PlatformP2PKHAddress::from_address_info(info) else {
            continue;
        };
        let key = keys
            .derive_private_key(&info.path)
            .map_err(|e| e.to_string())?;
        signer.add_address_key(address.to_bytes(), *key.to_bytes());
    }
    Ok(signer)
}

/// The id of the document an address withdrawal makes, as Drive makes it
/// (`address_credit_withdrawal/v0/transformer.rs`): from the first input in the transition's
/// order, its nonce, and the payout's script. `nonce` is the one the transition carries, which
/// is the address's nonce once it went through.
pub fn withdrawal_document_id(first: &PlatformAddress, nonce: u32, script: &[u8]) -> String {
    use dpp::data_contracts::withdrawals_contract;
    use dpp::data_contracts::withdrawals_contract::v1::document_types::withdrawal;
    let mut entropy = Vec::new();
    entropy.extend_from_slice(&nonce.to_be_bytes());
    entropy.extend_from_slice(&first.to_bytes());
    entropy.extend_from_slice(script);
    dpp::document::Document::generate_document_id_v0(
        &withdrawals_contract::ID,
        &withdrawals_contract::OWNER_ID,
        withdrawal::NAME,
        &entropy,
    )
    .to_string(Encoding::Base58)
}

impl Chain {
    /// The credits on each of the wallet's Platform addresses, in their own order, as of the
    /// last sync: what the form plans with.
    pub async fn address_inputs(&self, wallet: &PlatformWallet) -> Vec<u64> {
        let mut found = wallet.platform().addresses_with_balances().await;
        found.sort_by_key(|(address, _)| *address);
        found.into_iter().map(|(_, credits)| credits).collect()
    }

    /// Whether `address` is one of the wallet's own Core addresses, used or still to be (R-3).
    pub async fn owns_core_address(&self, wallet: &PlatformWallet, address: &Address) -> bool {
        wallet
            .state()
            .await
            .core_wallet
            .monitored_addresses()
            .contains(address)
    }

    /// The wallet's identity with the DPNS name `label`, when it is one of the wallet's (R-1а:
    /// a foreign name is not topped up from the addresses).
    pub async fn own_identity_named(
        &self,
        wallet: &PlatformWallet,
        label: &str,
    ) -> Option<Identifier> {
        let state = wallet.state().await;
        identities::of_wallet(&state.identity_manager, wallet.wallet_id())
            .find(|m| {
                m.dpns_names
                    .iter()
                    .any(|n| n.label.eq_ignore_ascii_case(label))
            })
            .map(|m| m.identity.id())
    }

    /// The same, proved now: what a payment plans with, so a stale sync gives neither a
    /// refusal nor another choice of addresses. What it proves goes into the wallet's own
    /// balances too (BUG-141): a stale one there — the form planned 0.00287511 on a fresh copy
    /// of the data, the proof said less — is corrected at once, so the tile, the snapshot and
    /// the form show what the core plans with, instead of «enough» on the form and «not
    /// enough» after the password.
    pub async fn address_balances_now(
        &self,
        wallet: &PlatformWallet,
    ) -> Result<Vec<(PlatformAddress, u64)>, String> {
        use dash_sdk::platform::FetchMany;
        use dash_sdk::query_types::{AddressInfo, AddressInfos};
        // The addresses the wallet knows funded, each read with proof now (as platform-wallet's
        // own spend path reads them: `AddressInfo::fetch_many`).
        let known: std::collections::BTreeSet<PlatformAddress> = wallet
            .platform()
            .addresses_with_balances()
            .await
            .into_iter()
            .map(|(address, _)| address)
            .collect();
        if known.is_empty() {
            return Ok(Vec::new());
        }
        let sdk = self.sdk().await?;
        let (proved, metadata): (AddressInfos, _) =
            AddressInfo::fetch_many_with_metadata(sdk.as_ref(), known, None)
                .await
                .map_err(|e| e.to_string())?;
        // Pinned at the proof's height, as platform-wallet does after its own transfer: a later
        // sync delta at or below it is not applied again on top.
        wallet
            .platform()
            .reconcile_address_infos(&proved, metadata.height, "balances proved before a payment")
            .await;
        let mut found: Vec<(PlatformAddress, u64)> = proved
            .into_iter()
            .map(|(address, info)| (address, info.map_or(0, |i| i.balance)))
            .collect();
        found.sort_by_key(|(address, _)| *address);
        Ok(found)
    }

    /// Pays from the addresses as `plan` says. `balances`: those the plan was made from
    /// ([`Self::address_balances_now`]), whose indexes it uses.
    pub async fn address_pay(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: AddressPayee,
        plan: &AddressPlan,
        balances: &[(PlatformAddress, u64)],
    ) -> Result<AddressPaid, String> {
        let Transition { inputs, payer_at } = transition(plan, balances);
        let before: u64 = inputs.keys().map(|a| balance_of(balances, a)).sum();
        let signer = address_signer(wallet, keys).await?;
        let platform = wallet.platform();
        let mut document = None;
        match to {
            AddressPayee::Platform(to) => {
                let (outputs, strategy) = transfer_outputs(&inputs, payer_at, to);
                platform
                    .transfer(
                        ACCOUNT,
                        InputSelection::Explicit(inputs.clone()),
                        outputs,
                        strategy,
                        None,
                        &signer,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
            }
            AddressPayee::Core(to) => {
                let script = to.script_pubkey();
                let at = payer_at.ok_or("A withdrawal pays its fee from an input")?;
                let changed = platform
                    .withdraw(
                        ACCOUNT,
                        InputSelection::Explicit(inputs.clone()),
                        CoreScript::new(script.clone()),
                        CORE_FEE_PER_BYTE,
                        vec![AddressFundsFeeStrategyStep::DeductFromInput(at as u16)],
                        None,
                        &signer,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                // The first input's nonce as the transition carried it: its nonce now.
                if let Some(first) = inputs.keys().next() {
                    let nonce = changed.addresses.iter().find_map(|entry| {
                        let bytes: [u8; 20] = entry.address.into();
                        (PlatformAddress::P2pkh(bytes) == *first).then_some(entry.funds.nonce)
                    });
                    document = nonce.map(|n| {
                        format!("wd:{}", withdrawal_document_id(first, n, script.as_bytes()))
                    });
                }
            }
            AddressPayee::Identity(id) => {
                // dpp takes a top-up's fee from its first input; the plan chose that one.
                wallet
                    .top_up_from_addresses(&id, inputs.clone(), &signer, None)
                    .await
                    .map_err(|e| e.to_string())?;
                let _ = wallet.identity().refresh_identity(&id).await;
            }
        }
        let now: Vec<(PlatformAddress, u64)> = wallet.platform().addresses_with_balances().await;
        let after: u64 = inputs.keys().map(|a| balance_of(&now, a)).sum();
        Ok(AddressPaid {
            before,
            after,
            document,
        })
    }
}

/// A payment's inputs as the transition keeps them (in the addresses' own order), and where
/// the fee comes from: the payer's place among them, `None` for «Max» of a transfer.
struct Transition {
    inputs: BTreeMap<PlatformAddress, u64>,
    payer_at: Option<usize>,
}

fn transition(plan: &AddressPlan, balances: &[(PlatformAddress, u64)]) -> Transition {
    let inputs: BTreeMap<PlatformAddress, u64> = plan
        .inputs
        .iter()
        .map(|&(i, credits)| (balances[i].0, credits))
        .collect();
    let payer_at = plan
        .fee_payer
        .and_then(|payer| inputs.keys().position(|a| *a == balances[payer].0));
    Transition { inputs, payer_at }
}

/// A transfer's outputs and fee strategy: the recipient alone, given what the inputs give; no
/// change output (the architect, 2026-11-10: what an address does not give stays on it). The
/// fee from the payer's rest, or, for «Max», out of the one output.
fn transfer_outputs(
    inputs: &BTreeMap<PlatformAddress, u64>,
    payer_at: Option<usize>,
    to: PlatformAddress,
) -> (
    BTreeMap<PlatformAddress, u64>,
    Vec<AddressFundsFeeStrategyStep>,
) {
    let given: u64 = inputs.values().sum();
    let strategy = match payer_at {
        Some(at) => vec![AddressFundsFeeStrategyStep::DeductFromInput(at as u16)],
        None => vec![AddressFundsFeeStrategyStep::ReduceOutput(0)],
    };
    (BTreeMap::from([(to, given)]), strategy)
}

fn balance_of(balances: &[(PlatformAddress, u64)], address: &PlatformAddress) -> u64 {
    balances
        .iter()
        .find(|(a, _)| a == address)
        .map_or(0, |(_, c)| *c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpp::state_transition::state_transitions::address_funds::address_credit_withdrawal_transition::AddressCreditWithdrawalTransition;
    use dpp::state_transition::state_transitions::address_funds::address_funds_transfer_transition::AddressFundsTransferTransition;
    use dpp::version::PlatformVersion;
    use evolite_api::address_plan as plan;

    /// The plan's fees are never below the protocol's minimums at every version evolite may
    /// meet (dpp's own estimates); since QA pass 21 they are above them, as Platform meters
    /// more (AC-T3). The limits are the version tables'.
    #[test]
    fn the_plans_fees_and_limits_are_the_protocols() {
        let version = PlatformVersion::latest();
        for inputs in 1..=plan::MAX_INPUTS {
            assert!(
                plan::AddressMove::Transfer.fee(inputs)
                    >= AddressFundsTransferTransition::estimate_min_fee(inputs, 1, version),
                "transfer, {inputs} inputs"
            );
            assert!(
                plan::AddressMove::Withdraw.fee(inputs)
                    >= AddressCreditWithdrawalTransition::estimate_min_fee(inputs, false, version),
                "withdrawal, {inputs} inputs"
            );
        }
        let min = &version.fee_version.state_transition_min_fees;
        assert!(
            plan::AddressMove::TopUp.fee(3)
                >= min.identity_topup_base_cost + 3 * min.address_funds_transfer_input_cost
        );
        let limits = &version.dpp.state_transitions.address_funds;
        assert_eq!(plan::MIN_INPUT, limits.min_input_amount);
        assert_eq!(plan::MIN_OUTPUT, limits.min_output_amount);
        assert_eq!(plan::MIN_FUNDING, limits.min_identity_funding_amount);
        assert_eq!(
            plan::MAX_INPUTS,
            version.dpp.state_transitions.max_address_inputs as usize
        );
        assert_eq!(
            plan::MIN_WITHDRAWAL,
            version.system_limits.min_withdrawal_amount
        );
    }

    /// The architect, 2026-11-10 (R-6 as amended): a partial spend makes no new address — its
    /// one output is the recipient, the inputs give only what the plan takes, and the fee comes
    /// from the largest's rest, by its place in the addresses' order.
    #[test]
    fn a_partial_spend_makes_no_change_output() {
        let a = PlatformAddress::P2pkh([1; 20]);
        let b = PlatformAddress::P2pkh([2; 20]);
        let to = PlatformAddress::P2pkh([9; 20]);
        let balances = [(a, 30_000_000), (b, 90_000_000)];
        let credits: Vec<u64> = balances.iter().map(|(_, c)| *c).collect();
        let plan = plan::address_plan(plan::AddressMove::Transfer, 100_000_000, false, &credits)
            .expect("a plan");
        let Transition { inputs, payer_at } = transition(&plan, &balances);
        let (outputs, strategy) = transfer_outputs(&inputs, payer_at, to);
        assert_eq!(outputs.len(), 1, "the recipient alone: {outputs:?}");
        assert_eq!(outputs.get(&to), Some(&100_000_000));
        assert_eq!(inputs.values().sum::<u64>(), 100_000_000);
        // The larger address pays the fee; it is second in the addresses' order.
        assert_eq!(payer_at, Some(1));
        assert_eq!(
            strategy,
            vec![AddressFundsFeeStrategyStep::DeductFromInput(1)]
        );
        assert!(inputs[&b] + plan.fee <= 90_000_000, "the fee is left on it");
        // «Max»: the output pays the fee.
        let max = plan::address_plan(plan::AddressMove::Transfer, 0, true, &credits).unwrap();
        let Transition { inputs, payer_at } = transition(&max, &balances);
        let (outputs, strategy) = transfer_outputs(&inputs, payer_at, to);
        assert_eq!((outputs.len(), outputs[&to]), (1, 120_000_000));
        assert_eq!(strategy, vec![AddressFundsFeeStrategyStep::ReduceOutput(0)]);
    }

    /// The document id is Drive's: the contract's owner, the first input's nonce and address,
    /// the payout script. A different nonce, address or script gives another id.
    #[test]
    fn a_withdrawals_document_id_follows_its_inputs() {
        let a = PlatformAddress::P2pkh([1; 20]);
        let b = PlatformAddress::P2pkh([2; 20]);
        let id = withdrawal_document_id(&a, 5, &[0x76, 0xa9]);
        assert_eq!(id, withdrawal_document_id(&a, 5, &[0x76, 0xa9]));
        assert_ne!(id, withdrawal_document_id(&a, 6, &[0x76, 0xa9]));
        assert_ne!(id, withdrawal_document_id(&b, 5, &[0x76, 0xa9]));
        assert_ne!(id, withdrawal_document_id(&a, 5, &[0x76]));
    }
}
