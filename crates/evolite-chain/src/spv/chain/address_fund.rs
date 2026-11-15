//! Funding the wallet's own Platform address from Core (FEAT-0015 stage 2): an asset lock from
//! the `AssetLockAddressTopUp` account, then an `AddressFundingFromAssetLock` transition with one
//! output, the address, and no amount (`None`): the address gets the lock less Platform's fee.
//!
//! platform-wallet keeps the lock in its own store from the moment it is built until Platform
//! takes it (`consume_asset_lock`), so a funding cut short after the lock (a crash, a lock of
//! the wallet, the network) is found again — here and after a restart — and finished from the
//! same lock, without new money (R-8, as BUG-7 does for an identity's top-up).

use super::*;

/// The Platform payment account the address belongs to (DIP-17 account 0).
const ACCOUNT: u32 = 0;

/// The one output of the transition: the address gets all the lock leaves after the fee
/// (AC-T3). No explicit amounts, so nothing burns and nothing is left for a second output.
pub fn fund_outputs(to: PlatformAddress) -> BTreeMap<PlatformAddress, Option<u64>> {
    BTreeMap::from([(to, None)])
}

/// A lock that funds an address and that the wallet has not seen taken (not consumed): made
/// here and cut short (`made_here`), or found on chain after a restore from the phrase (AC-T6).
/// Whether Platform took it is not known from the wallet: QA closed the window after Platform
/// accepted a funding and before platform-wallet marked its lock consumed (BUG-160).
/// [`waiting_lock`] goes by Platform's proved answer (FEAT-0016).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FundLock {
    pub txid: String,
    pub out_point: key_wallet::dashcore::OutPoint,
    pub amount: u64,
    pub made_here: bool,
    /// Its proof is an InstantLock (BUG-171). Without one — found on chain after a restore, or
    /// its InstantLock never came — Platform gets a ChainLock proof and keeps more of it.
    pub instant: bool,
}

/// The credits a funding brought the address: its balance after less the one before, both
/// proved (BUG-171 AC-T4); `None` when either is not known.
pub fn credited(before: Option<u64>, after: Option<u64>) -> Option<u64> {
    Some(after?.saturating_sub(before?))
}

/// Whether a tracked lock of `funding` in `status` funds an address and is not seen taken.
fn open(
    funding: platform_wallet::AssetLockFundingType,
    status: &platform_wallet::AssetLockStatus,
) -> bool {
    funding == platform_wallet::AssetLockFundingType::AssetLockAddressTopUp && !status.is_consumed()
}

/// Of the open locks, the one to offer «Continue» for (AC-T5, AC-T6, BUG-160), the first such
/// in the order given:
/// - one Platform proved it has not seen (`NotSeen`);
/// - one made here that Platform has not answered about yet (its steps show while it runs).
///
/// One Platform used, whole or in part, is closed: «Continue» would only be refused. A
/// recovered one not answered yet is not offered until it is: an answer is never guessed.
pub fn waiting_lock(
    locks: &[FundLock],
    answer: impl Fn(&str) -> Option<crate::lock_status::Answer>,
) -> Option<&FundLock> {
    use crate::lock_status::Answer;
    locks.iter().find(|l| match answer(&l.txid) {
        Some(Answer::NotSeen) => true,
        Some(Answer::Spent | Answer::Partly { .. }) => false,
        None => l.made_here,
    })
}

impl Chain {
    /// The locks that fund an address and are not seen taken: made here or recovered from the
    /// chain (see [`FundLock`]).
    pub async fn open_fund_locks(&self, wallet: &PlatformWallet) -> Vec<FundLock> {
        wallet
            .asset_locks()
            .list_tracked_locks()
            .await
            .into_iter()
            .filter(|l| open(l.funding_type, &l.status))
            .map(|l| FundLock {
                txid: l.out_point.txid.to_string(),
                out_point: l.out_point,
                amount: l.amount,
                made_here: l.status != platform_wallet::AssetLockStatus::RecoveredFromChain,
                instant: matches!(l.proof, Some(dpp::prelude::AssetLockProof::Instant(_))),
            })
            .collect()
    }

    /// The asset locks made to fund the address, finished or not, by their Core transaction
    /// (txid) with the duffs each locked: the history names them «Transfer from Core to the
    /// Platform address», with what left Core (AC-T7).
    pub async fn address_fund_locks(
        &self,
        wallet: &PlatformWallet,
    ) -> std::collections::HashMap<String, u64> {
        wallet
            .asset_locks()
            .list_tracked_locks()
            .await
            .into_iter()
            .filter(|l| {
                l.funding_type == platform_wallet::AssetLockFundingType::AssetLockAddressTopUp
            })
            .map(|l| (l.out_point.txid.to_string(), l.amount))
            .collect()
    }

    /// The first half of a funding (BUG-171): a lock of `lock` duffs for the address is built,
    /// broadcast, proved (InstantSend, else — after platform-wallet's wait for it — on its way
    /// to the ChainLock) and stored, and its out point returned, for the caller to record where
    /// it goes before Platform is asked ([`Self::fund_address`] from it), and whether it got an
    /// InstantLock (else Platform gets a ChainLock proof). QA's `--stop-after asset-lock` ends
    /// the process here.
    pub async fn lock_for_address(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        lock: u64,
    ) -> Result<(key_wallet::dashcore::OutPoint, bool), String> {
        let lock_signer = crate::identity::AssetLockSigner(keys.clone());
        let (out_point, instant) = match wallet
            .asset_locks()
            .create_funded_asset_lock_proof(
                lock,
                0,
                platform_wallet::AssetLockFundingType::AssetLockAddressTopUp,
                // As `fund_from_asset_lock` asks for it: destination index 0.
                0,
                &lock_signer,
            )
            .await
        {
            Ok((proof, _, out_point)) => (
                out_point,
                matches!(proof, dpp::prelude::AssetLockProof::Instant(_)),
            ),
            // No InstantLock in time: the lock is on chain and tracked; funding from it waits
            // for its ChainLock (platform-wallet's own fallback).
            Err(platform_wallet::PlatformWalletError::FinalityTimeout(out_point)) => {
                (out_point, false)
            }
            Err(e) => return Err(e.to_string()),
        };
        // The store buffers its writes: on disk before Platform is asked or the process ends.
        wallet.flush_persist().map_err(|e| e.to_string())?;
        Ok((out_point, instant))
    }

    /// Credits `to`, one of the wallet's own Platform addresses, from the lock at `out_point`
    /// (made by [`Self::lock_for_address`], or a cut-short funding's): no new money. Waits for
    /// InstantSend or the ChainLock, however long that takes (platform-wallet's own rule: a
    /// lock on chain is final), so it runs in the background. The answer: the credits the
    /// address gained, by its proved balance before and the one the transition proved after
    /// (BUG-171: «Credited» says what came, not what was planned); `None` when either is not
    /// known.
    pub async fn fund_address(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: PlatformAddress,
        out_point: key_wallet::dashcore::OutPoint,
    ) -> Result<Option<u64>, String> {
        let before = self.proved_balance(to).await;
        let funding = platform_wallet::AssetLockFunding::FromExistingAssetLock {
            out_point,
            consume_invitation_voucher: false,
        };
        let address_signer = super::addresses::address_signer(wallet, keys).await?;
        let lock_signer = crate::identity::AssetLockSigner(keys.clone());
        let changes = wallet
            .platform()
            .fund_from_asset_lock(
                funding,
                ACCOUNT,
                fund_outputs(to),
                Vec::new(),
                &address_signer,
                &lock_signer,
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
        let after = changes
            .addresses
            .iter()
            .rev()
            .find(|e| PlatformAddress::P2pkh(e.address.to_bytes()) == to)
            .map(|e| e.funds.balance);
        Ok(credited(before, after))
    }

    /// `to`'s credits as Platform proves them now (none: an address it has not seen), or
    /// `None` when the read fails.
    async fn proved_balance(&self, to: PlatformAddress) -> Option<u64> {
        use dash_sdk::platform::FetchMany;
        use dash_sdk::query_types::{AddressInfo, AddressInfos};
        let sdk = self.sdk().await.ok()?;
        let proved: AddressInfos =
            AddressInfo::fetch_many(sdk.as_ref(), std::collections::BTreeSet::from([to]))
                .await
                .ok()?;
        Some(
            proved
                .get(&to)
                .and_then(|info| info.as_ref())
                .map_or(0, |i| i.balance),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock_status::Answer;
    use platform_wallet::{AssetLockFundingType as Funding, AssetLockStatus as Status};

    /// AC-T3: one output, the address, with no amount — the rest after the fee goes to it.
    #[test]
    fn the_address_gets_all_the_lock_leaves() {
        let to = PlatformAddress::P2pkh([7; 20]);
        let outputs = fund_outputs(to);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs.get(&to), Some(&None));
    }

    /// AC-T5, T6: only a lock for an address and not seen taken is open — made here or found
    /// on chain; an identity's or the pool's is not, nor one consumed.
    #[test]
    fn only_an_address_lock_not_taken_is_open() {
        assert!(open(Funding::AssetLockAddressTopUp, &Status::Broadcast));
        assert!(open(
            Funding::AssetLockAddressTopUp,
            &Status::InstantSendLocked
        ));
        assert!(open(
            Funding::AssetLockAddressTopUp,
            &Status::RecoveredFromChain
        ));
        assert!(!open(Funding::AssetLockAddressTopUp, &Status::Consumed));
        assert!(!open(Funding::IdentityTopUp, &Status::Broadcast));
        assert!(!open(
            Funding::AssetLockShieldedAddressTopUp,
            &Status::Broadcast
        ));
    }

    fn lock(n: u8, made_here: bool) -> FundLock {
        use key_wallet::dashcore::hashes::Hash;
        FundLock {
            txid: format!("{n:02x}").repeat(32),
            out_point: key_wallet::dashcore::OutPoint::new(
                key_wallet::dashcore::Txid::from_byte_array([n; 32]),
                0,
            ),
            amount: 10_012_000,
            made_here,
            instant: false,
        }
    }

    /// Answers substituted for Platform's, one per lock in order.
    fn told<const N: usize>(
        locks: &[FundLock; N],
        answers: [Option<Answer>; N],
    ) -> impl Fn(&str) -> Option<Answer> {
        let txids: Vec<String> = locks.iter().map(|l| l.txid.clone()).collect();
        move |txid: &str| answers[txids.iter().position(|t| t == txid)?]
    }

    /// AC-T6: after a restore, a lock found on chain is offered to continue only when Platform
    /// proved it has not seen it; one it used, whole or in part, is not, and one not answered
    /// yet is not offered until it is.
    #[test]
    fn a_recovered_lock_is_unfinished_only_when_platform_has_not_seen_it() {
        let locks = [lock(1, false), lock(2, false), lock(3, false)];
        // Spent, partly used, not answered: nothing to continue.
        let answers = [Some(Answer::Spent), Some(Answer::Partly { left: 5 }), None];
        assert_eq!(waiting_lock(&locks, told(&locks, answers)), None);
        // One Platform has not seen: that one.
        let answers = [Some(Answer::Spent), None, Some(Answer::NotSeen)];
        assert_eq!(waiting_lock(&locks, told(&locks, answers)), Some(&locks[2]));
        assert_eq!(waiting_lock(&[], |_| Some(Answer::NotSeen)), None);
    }

    /// BUG-160: the window was closed after Platform accepted the funding and before the
    /// wallet marked its lock consumed. Once Platform proves it used the lock, it waits for no
    /// «Continue»; until Platform answers, or when it proves it has not seen it, it does.
    #[test]
    fn a_lock_made_here_that_platform_used_is_not_waiting() {
        let locks = [lock(1, true)];
        assert_eq!(
            waiting_lock(&locks, told(&locks, [Some(Answer::Spent)])),
            None
        );
        assert_eq!(
            waiting_lock(&locks, told(&locks, [Some(Answer::Partly { left: 5 })])),
            None
        );
        assert_eq!(waiting_lock(&locks, told(&locks, [None])), Some(&locks[0]));
        assert_eq!(
            waiting_lock(&locks, told(&locks, [Some(Answer::NotSeen)])),
            Some(&locks[0])
        );
        // A used one ahead of an unseen one does not hide it.
        let locks = [lock(1, true), lock(2, false)];
        let answers = [Some(Answer::Spent), Some(Answer::NotSeen)];
        assert_eq!(waiting_lock(&locks, told(&locks, answers)), Some(&locks[1]));
    }
}

#[cfg(test)]
mod credited_tests {
    use super::*;

    /// BUG-171 AC-T4: «Credited» is the proved balance after less the one before — 97 064 000
    /// credits on QA's C2, not the planned 100 000 000; unknown when either read is.
    #[test]
    fn credited_is_after_less_before() {
        assert_eq!(
            credited(Some(492_925_000), Some(589_989_000)),
            Some(97_064_000)
        );
        assert_eq!(credited(Some(0), Some(103_040_000)), Some(103_040_000));
        assert_eq!(credited(None, Some(1)), None);
        assert_eq!(credited(Some(1), None), None);
    }
}
