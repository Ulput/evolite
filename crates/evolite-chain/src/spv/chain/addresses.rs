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
) -> Result<crate::key_signer::KeySigner, String> {
    address_signer_of(wallet, keys, ACCOUNT).await
}

/// [`address_signer`] of the DIP-17 payment account `account` (key class 0): FEAT-0048's
/// refusal account is 1.
pub(crate) async fn address_signer_of(
    wallet: &PlatformWallet,
    keys: &key_wallet::Wallet,
    account: u32,
) -> Result<crate::key_signer::KeySigner, String> {
    let state = wallet.state().await;
    let account = state
        .core_wallet
        .platform_payment_managed_account_at_index(account)
        .ok_or(tr!("The wallet has no Platform addresses"))?;
    let mut signer = crate::key_signer::KeySigner::default();
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
    /// Each of the wallet's Platform addresses (bech32m) with its credits, in their own order,
    /// as of the last sync: what the form plans with (a transfer leaves its recipient out,
    /// BUG-141).
    pub async fn address_inputs(&self, wallet: &PlatformWallet) -> Vec<(String, u64)> {
        let mut found = wallet.platform().addresses_with_balances().await;
        found.sort_by_key(|(address, _)| *address);
        found
            .into_iter()
            .map(|(address, credits)| (address.to_bech32m_string(crate::network::dash()), credits))
            .collect()
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

    /// FEAT-0048: gives the open wallet the refusal account (DIP-17 account 1, key class 0)
    /// when it lacks it. The order is platform-wallet's for a DashPay account (contacts.rs):
    /// the registration goes to the store first, so a failed write changes nothing, and the
    /// next load has the account before any of its address or balance rows (the store refuses
    /// rows it cannot place). Then the account goes into the wallet and its managed info, then
    /// the address provider is rebuilt so the sync watches it. Nothing changes when the account
    /// is there already; a wallet born before FEAT-0048 gets it at its next open
    /// ([`Self::open_wallet`]).
    pub async fn ensure_refusal_account(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<(), String> {
        if self.add_refusal_account(wallet, keys).await? {
            wallet
                .platform()
                .add_provider(REFUSAL_ACCOUNT)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// FEAT-0048: where Platform returns the money when it refuses to create the identity at
    /// DIP-9 slot `slot`: the refusal account's address at index `slot`. It comes from the
    /// account's own address pool, so it is the address the sync watches; a slot past the
    /// pool's end extends the pool (and the provider) to it. Account 0, the one «Receive»
    /// shows, is not touched.
    pub async fn refusal_address(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        slot: u32,
    ) -> Result<PlatformAddress, String> {
        let added = self.add_refusal_account(wallet, keys).await?;
        let (address, extended) = {
            let mut manager = wallet.wallet_manager().write().await;
            let (held, info) = manager
                .get_wallet_mut_and_info_mut(&wallet.wallet_id())
                .ok_or(tr!("The wallet is not open"))?;
            let xpub = held
                .accounts
                .platform_payment_accounts
                .get(&REFUSAL_KEY)
                .map(|a| a.account_xpub)
                .ok_or("No refusal account")?;
            let pool = &mut info
                .core_wallet
                .platform_payment_managed_account_mut(REFUSAL_ACCOUNT, 0)
                .ok_or("No refusal account")?
                .addresses;
            let extended = pool.address_at_index(slot).is_none();
            if extended {
                let source = key_wallet::KeySource::Public(xpub);
                pool.address_range(0, slot + 1, &source)
                    .map_err(|e| e.to_string())?;
            }
            let address = pool
                .address_at_index(slot)
                .ok_or("No refusal address at this slot")?;
            (address, extended)
        };
        if added || extended {
            wallet
                .platform()
                .add_provider(REFUSAL_ACCOUNT)
                .await
                .map_err(|e| e.to_string())?;
        }
        PlatformAddress::try_from(address).map_err(|e| e.to_string())
    }

    /// FEAT-0048 r1 Must 2: the credits on `address` now, read with proof (`0` when Platform
    /// holds none there). An `Err` when it could not be read: what starts from it waits.
    pub async fn address_credits_proved(&self, address: PlatformAddress) -> Result<u64, String> {
        use dash_sdk::platform::Fetch;
        use dash_sdk::query_types::AddressInfo;
        let sdk = self.sdk().await?;
        let info = AddressInfo::fetch(sdk.as_ref(), address)
            .await
            .map_err(|e| e.to_string())?;
        Ok(info.map_or(0, |info| info.balance))
    }

    /// FEAT-0048: the credits on slot `slot`'s refusal address as of the last sync of the
    /// wallet's Platform addresses (the refusal account is synced as account 0 is). `None`: the
    /// account or that address is not there.
    pub async fn refusal_credits(&self, wallet: &PlatformWallet, slot: u32) -> Option<u64> {
        let manager = wallet.wallet_manager().read().await;
        let account = manager
            .get_wallet_info(&wallet.wallet_id())?
            .core_wallet
            .platform_payment_managed_account(REFUSAL_ACCOUNT, 0)?;
        let wanted = PlatformAddress::try_from(account.addresses.address_at_index(slot)?).ok()?;
        Some(
            account
                .address_balances
                .iter()
                .filter(|(p2pkh, _)| PlatformAddress::P2pkh(p2pkh.to_bytes()) == wanted)
                .map(|(_, credits)| *credits)
                .sum(),
        )
    }

    /// FEAT-0048: [`Self::ensure_refusal_account`] at an open, before the client syncs the
    /// wallet's Platform addresses. The keys are made only when the account is missing (once
    /// per wallet: the store keeps it after). A failure does not stop the open: the account is
    /// tried again when a refusal address is asked for.
    pub(super) async fn open_refusal_account(&self, wallet: &PlatformWallet, seed: &[u8; 64]) {
        let has = {
            let state = wallet.state().await;
            state
                .core_wallet
                .has_platform_payment_account(REFUSAL_ACCOUNT, 0)
        };
        if has {
            return;
        }
        let ensured = match crate::keys_from(seed) {
            Ok(keys) => self.ensure_refusal_account(wallet, &keys).await,
            Err(e) => Err(format!("{e:?}")),
        };
        if let Err(e) = ensured {
            crate::diag::platform_failed("ensure_refusal_account", &e);
        }
    }

    /// FEAT-0048: stores the refusal account's registration, then puts the account into both
    /// halves of the open wallet the way key-wallet builds them at birth: the account (its xpub
    /// from `keys`) into the wallet's accounts, its address pool into the managed info.
    /// platform-wallet has no call for a Platform payment account. Whether it added anything.
    async fn add_refusal_account(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
    ) -> Result<bool, String> {
        use key_wallet::managed_account::managed_account_collection::ManagedAccountCollection;
        use platform_wallet::changeset::{
            AccountAddressPoolEntry, AccountRegistrationEntry, PlatformWalletChangeSet,
            PlatformWalletPersistence,
        };
        // Held from the check to the inserts: two opens at once add it once.
        let mut manager = wallet.wallet_manager().write().await;
        let (held, info) = manager
            .get_wallet_mut_and_info_mut(&wallet.wallet_id())
            .ok_or(tr!("The wallet is not open"))?;
        let has_account = held
            .accounts
            .platform_payment_accounts
            .contains_key(&REFUSAL_KEY);
        let has_managed = info
            .core_wallet
            .has_platform_payment_account(REFUSAL_ACCOUNT, 0);
        if has_account && has_managed {
            return Ok(false);
        }
        // Keys of another wallet would give addresses nobody here can spend.
        if keys.wallet_id != held.wallet_id {
            return Err("The keys are not this wallet's".into());
        }
        let kind = key_wallet::AccountType::PlatformPayment {
            account: REFUSAL_ACCOUNT,
            key_class: 0,
        };
        let account = match held.accounts.platform_payment_accounts.get(&REFUSAL_KEY) {
            Some(account) => account.clone(),
            None => {
                let path = kind
                    .derivation_path(held.network)
                    .map_err(|e| e.to_string())?;
                let xpub = keys
                    .derive_extended_public_key(&path)
                    .map_err(|e| e.to_string())?;
                key_wallet::Account::new(Some(held.wallet_id), kind, xpub, held.network)
                    .map_err(|e| e.to_string())?
            }
        };
        // key-wallet's own conversion makes the pool (DIP-17 gap, the account's xpub): the one
        // it makes for account 0 at birth.
        let mut one = key_wallet::AccountCollection::new();
        one.insert(account.clone()).map_err(|e| e.to_string())?;
        let managed = ManagedAccountCollection::from_account_collection(&one)
            .platform_payment_accounts
            .remove(&REFUSAL_KEY)
            .ok_or("The refusal account's addresses were not made")?;
        // The rows a birth writes for a Platform payment account (wallet_lifecycle.rs): its
        // registration and its pool.
        let changeset = PlatformWalletChangeSet {
            account_registrations: vec![AccountRegistrationEntry {
                account_type: kind,
                account_xpub: account.account_xpub,
            }],
            account_address_pools: vec![AccountAddressPoolEntry {
                account_type: kind,
                pool_type: managed.addresses.pool_type,
                addresses: managed.addresses.addresses.values().cloned().collect(),
            }],
            ..Default::default()
        };
        let store = self
            .persister
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or("The wallet's store is not open")?;
        store
            .store(wallet.wallet_id(), changeset)
            .map_err(|e| format!("{e:?}"))?;
        if !has_account {
            held.add_account(kind, Some(account.account_xpub))
                .map_err(|e| e.to_string())?;
        }
        if !has_managed {
            info.core_wallet.accounts.insert_platform_account(managed);
        }
        Ok(true)
    }
}

/// FEAT-0048: the Platform account a refused private identity creation pays back to (the
/// architect: account 1, key class 0, index = the identity's slot). «Receive» shows account 0
/// only, so these addresses are never handed out for receiving.
pub(crate) const REFUSAL_ACCOUNT: u32 = 1;
const REFUSAL_KEY: key_wallet::account::account_collection::PlatformPaymentAccountKey =
    key_wallet::account::account_collection::PlatformPaymentAccountKey {
        account: REFUSAL_ACCOUNT,
        key_class: 0,
    };

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
pub(super) mod tests {
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

    pub(in crate::spv::chain) const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    /// The store's paths in the folder `name` (emptied first when `fresh`).
    pub(in crate::spv::chain) fn store_paths(name: &str, fresh: bool) -> ChainPaths {
        let dir = std::env::temp_dir().join(format!("evolite-{name}-{}", std::process::id()));
        if fresh {
            let _ = std::fs::remove_dir_all(&dir);
        }
        ChainPaths {
            spv: dir.join("spv").join("testnet"),
            store: dir
                .join("wallets")
                .join("testnet")
                .join("platform-wallet.db"),
            core_node: dir.join("core-node.txt"),
        }
    }

    /// A chain on the store only (no SPV client, no network, so no sync), with the public test
    /// phrase's wallet open, and its keys.
    pub(in crate::spv::chain) async fn open_on(
        paths: &ChainPaths,
    ) -> (Chain, Arc<PlatformWallet>, key_wallet::Wallet) {
        let chain = Chain::default();
        chain.open_store_only(paths).await.expect("the store opens");
        let (_, wallet) = chain
            .open_wallet(&crate::seed_from(PHRASE, "").unwrap(), 0)
            .await
            .expect("wallet opens");
        (chain, wallet, crate::keys_of(PHRASE).expect("keys"))
    }

    /// What says the refusal account is there: in the wallet's accounts (its xpub) and in the
    /// managed info (its pool's addresses).
    async fn refusal_account(
        wallet: &PlatformWallet,
    ) -> Option<(key_wallet::bip32::ExtendedPubKey, Vec<Address>)> {
        let manager = wallet.wallet_manager().read().await;
        let (held, info) = manager.get_wallet_and_info(&wallet.wallet_id())?;
        let xpub = held
            .accounts
            .platform_payment_accounts
            .get(&REFUSAL_KEY)?
            .account_xpub;
        let pool = info
            .core_wallet
            .platform_payment_managed_account(REFUSAL_ACCOUNT, 0)?
            .addresses
            .all_addresses();
        Some((xpub, pool))
    }

    async fn receive_address(wallet: &PlatformWallet) -> PlatformAddress {
        wallet
            .platform()
            .next_unused_receive_address(PlatformPaymentAccountKey {
                account: 0,
                key_class: 0,
            })
            .await
            .expect("account 0's address")
    }

    /// The DIP-17 address m/9'/1'/17'/1'/0'/`slot`, derived from the keys alone.
    fn dip17_address(keys: &key_wallet::Wallet, slot: u32) -> Address {
        let path: DerivationPath = format!("m/9'/1'/17'/1'/0'/{slot}").parse().unwrap();
        let key = keys.derive_public_key(&path).unwrap();
        Address::p2pkh(
            &key_wallet::dashcore::PublicKey::new(key),
            crate::network::dash(),
        )
    }

    /// FEAT-0048: the refusal account is stored with the wallet: after a restart the store
    /// loads (it refuses rows it cannot place) with the account in it, and a second ensure
    /// changes nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_refusal_account_survives_a_restart() {
        let paths = store_paths("refusal-restart", true);
        let (chain, wallet, keys) = open_on(&paths).await;
        let id = wallet.wallet_id();
        let first = refusal_account(&wallet).await.expect("the open added it");
        chain.ensure_refusal_account(&wallet, &keys).await.unwrap();
        assert_eq!(refusal_account(&wallet).await, Some(first.clone()));
        drop(wallet);
        chain.stop().await;

        let again = Chain::default();
        again
            .open_store_only(&paths)
            .await
            .expect("the store loads");
        let loaded = again
            .wallet(&id)
            .await
            .expect("the wallet comes from the store");
        assert_eq!(refusal_account(&loaded).await, Some(first.clone()));
        again.ensure_refusal_account(&loaded, &keys).await.unwrap();
        assert_eq!(refusal_account(&loaded).await, Some(first));
        let (_, opened) = again
            .open_wallet(&crate::seed_from(PHRASE, "").unwrap(), 0)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&opened, &loaded), "not made again");
        again.stop().await;
    }

    /// FEAT-0048: credits on a refusal address are there after a restart, before any sync. The
    /// balance goes in as the address sync writes one (a `PlatformAddressChangeSet` row through
    /// the wallet's store); the load gives it back to account 1.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_refusal_balance_survives_a_restart() {
        use platform_wallet::changeset::{
            PlatformAddressBalanceEntry, PlatformAddressChangeSet, PlatformWalletChangeSet,
            PlatformWalletPersistence,
        };
        const SLOT: u32 = 3;
        const CREDITS: u64 = 1_234_567;
        let paths = store_paths("refusal-balance", true);
        let (chain, wallet, keys) = open_on(&paths).await;
        let id = wallet.wallet_id();
        let address = chain.refusal_address(&wallet, &keys, SLOT).await.unwrap();
        let p2pkh =
            key_wallet::PlatformP2PKHAddress::from_address(&dip17_address(&keys, SLOT)).unwrap();
        let store = chain.persister.lock().unwrap().clone().expect("the store");
        store
            .store(
                id,
                PlatformWalletChangeSet {
                    platform_addresses: Some(PlatformAddressChangeSet {
                        addresses: vec![PlatformAddressBalanceEntry {
                            wallet_id: id,
                            account_index: REFUSAL_ACCOUNT,
                            address_index: SLOT,
                            address: p2pkh,
                            funds: dash_sdk::platform::address_sync::AddressFunds {
                                nonce: 0,
                                balance: CREDITS,
                                as_of_height: 10,
                            },
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .expect("stored");
        drop(store);
        drop(wallet);
        chain.stop().await;

        let again = Chain::default();
        again
            .open_store_only(&paths)
            .await
            .expect("the store loads");
        let loaded = again
            .wallet(&id)
            .await
            .expect("the wallet comes from the store");
        let credits = loaded
            .state()
            .await
            .core_wallet
            .platform_payment_managed_account(REFUSAL_ACCOUNT, 0)
            .expect("the refusal account")
            .address_credit_balance(&p2pkh);
        assert_eq!(credits, CREDITS);
        assert_eq!(
            again.refusal_address(&loaded, &keys, SLOT).await.unwrap(),
            address
        );
        again.stop().await;
    }

    /// FEAT-0048: each slot has its own refusal address, none is the one «Receive» shows, and
    /// asking for them leaves account 0 as it was. The address is the DIP-17 one
    /// (m/9'/1'/17'/1'/0'/slot), also past the pool's first 20.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_refusal_address_is_per_slot_and_not_account_0s() {
        let (chain, wallet, keys) = open_on(&store_paths("refusal-slots", true)).await;
        let receive = receive_address(&wallet).await;
        let first = chain.refusal_address(&wallet, &keys, 0).await.unwrap();
        let second = chain.refusal_address(&wallet, &keys, 1).await.unwrap();
        let far = chain.refusal_address(&wallet, &keys, 25).await.unwrap();
        assert_ne!(first, second);
        assert_ne!(first, receive);
        assert_ne!(second, receive);
        assert_eq!(receive_address(&wallet).await, receive);
        assert_eq!(
            chain.refusal_address(&wallet, &keys, 0).await.unwrap(),
            first
        );
        for (slot, address) in [(0, first), (1, second), (25, far)] {
            let derived = PlatformAddress::try_from(dip17_address(&keys, slot)).unwrap();
            assert_eq!(derived, address, "slot {slot}");
        }
        chain.stop().await;
    }

    /// FEAT-0048: a restore in a new folder has the refusal account as soon as the wallet is
    /// open (no sync runs on the store alone), with the same addresses.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restored_wallet_has_the_refusal_account_before_any_sync() {
        let (chain, wallet, keys) = open_on(&store_paths("refusal-a", true)).await;
        let (again, restored, keys_again) = open_on(&store_paths("refusal-b", true)).await;
        assert!(refusal_account(&restored).await.is_some());
        assert_eq!(
            refusal_account(&restored).await,
            refusal_account(&wallet).await
        );
        for slot in [0, 3] {
            assert_eq!(
                chain.refusal_address(&wallet, &keys, slot).await.unwrap(),
                again
                    .refusal_address(&restored, &keys_again, slot)
                    .await
                    .unwrap()
            );
        }
        chain.stop().await;
        again.stop().await;
    }
}
