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

/// A funding of an address from Core that did not finish: the lock platform-wallet tracks for
/// it, made here and not yet taken by Platform. A lock found on chain after a restore is not
/// one (whether Platform took it is unknown; FEAT-0016 tells).
async fn unfinished_lock(wallet: &PlatformWallet) -> Option<platform_wallet::TrackedAssetLock> {
    wallet
        .asset_locks()
        .list_tracked_locks()
        .await
        .into_iter()
        .find(|l| unfinished(l.funding_type, &l.status))
}

/// Whether a tracked lock of `funding` in `status` is an unfinished funding of an address.
fn unfinished(
    funding: platform_wallet::AssetLockFundingType,
    status: &platform_wallet::AssetLockStatus,
) -> bool {
    funding == platform_wallet::AssetLockFundingType::AssetLockAddressTopUp
        && !status.is_consumed()
        && *status != platform_wallet::AssetLockStatus::RecoveredFromChain
}

/// The one output of the transition: the address gets all the lock leaves after the fee
/// (AC-T3). No explicit amounts, so nothing burns and nothing is left for a second output.
pub fn fund_outputs(to: PlatformAddress) -> BTreeMap<PlatformAddress, Option<u64>> {
    BTreeMap::from([(to, None)])
}

/// A lock that funds an address, found on chain after a restore from the phrase (FEAT-0015
/// AC-T6): its txid, out point and the duffs it locked. Whether Platform took it is not known
/// from the wallet; [`recovered_unfinished`] goes by Platform's proved answer (FEAT-0016).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveredLock {
    pub txid: String,
    pub out_point: key_wallet::dashcore::OutPoint,
    pub amount: u64,
}

/// Of the recovered locks, the one to offer «Continue» for (AC-T6): one Platform proved it has
/// not seen (`NotSeen`). One it used — whole or in part — is no unfinished funding; one not
/// answered yet is not offered until it is (an answer is never guessed). The first such, in
/// the order given.
pub fn recovered_unfinished(
    locks: &[RecoveredLock],
    answer: impl Fn(&str) -> Option<crate::lock_status::Answer>,
) -> Option<&RecoveredLock> {
    locks
        .iter()
        .find(|l| answer(&l.txid) == Some(crate::lock_status::Answer::NotSeen))
}

impl Chain {
    /// The locks that fund an address found on chain after a restore (AC-T6), not taken as far
    /// as the wallet knows: `RecoveredFromChain`, never consumed here.
    pub async fn recovered_fund_locks(&self, wallet: &PlatformWallet) -> Vec<RecoveredLock> {
        wallet
            .asset_locks()
            .list_tracked_locks()
            .await
            .into_iter()
            .filter(|l| {
                l.funding_type == platform_wallet::AssetLockFundingType::AssetLockAddressTopUp
                    && l.status == platform_wallet::AssetLockStatus::RecoveredFromChain
            })
            .map(|l| RecoveredLock {
                txid: l.out_point.txid.to_string(),
                out_point: l.out_point,
                amount: l.amount,
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

    /// How many duffs an unfinished funding of the address has locked, if one is waiting.
    pub async fn unfinished_address_fund(&self, wallet: &PlatformWallet) -> Option<u64> {
        unfinished_lock(wallet).await.map(|l| l.amount)
    }

    /// Credits `to`, one of the wallet's own Platform addresses, from Core: a new lock of
    /// `lock` duffs, or — when one is waiting — the unfinished one, without new money (the
    /// caller decides which with [`Self::unfinished_address_fund`]: `lock` is then ignored).
    /// `recovered`: a lock found on chain after a restore that Platform proved unseen (AC-T6),
    /// finished in the same way. Waits for InstantSend or the ChainLock, however long that takes
    /// (platform-wallet's own rule: a lock on chain is final), so it runs in the background.
    pub async fn fund_address(
        &self,
        wallet: &PlatformWallet,
        keys: &key_wallet::Wallet,
        to: PlatformAddress,
        lock: u64,
        recovered: Option<key_wallet::dashcore::OutPoint>,
    ) -> Result<(), String> {
        let waiting = unfinished_lock(wallet)
            .await
            .map(|l| l.out_point)
            .or(recovered);
        let funding = match waiting {
            Some(out_point) => platform_wallet::AssetLockFunding::FromExistingAssetLock {
                out_point,
                consume_invitation_voucher: false,
            },
            None => platform_wallet::AssetLockFunding::FromWalletBalance {
                amount_duffs: lock,
                account_index: 0,
            },
        };
        let address_signer = super::addresses::address_signer(wallet, keys).await?;
        let lock_signer = crate::identity::AssetLockSigner(keys.clone());
        wallet
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
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform_wallet::{AssetLockFundingType as Funding, AssetLockStatus as Status};

    /// AC-T3: one output, the address, with no amount — the rest after the fee goes to it.
    #[test]
    fn the_address_gets_all_the_lock_leaves() {
        let to = PlatformAddress::P2pkh([7; 20]);
        let outputs = fund_outputs(to);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs.get(&to), Some(&None));
    }

    /// AC-T5, T6: only a lock made here for an address and not yet taken is unfinished; an
    /// identity's or the pool's is not, nor one Platform took, nor one only found on chain.
    #[test]
    fn only_an_address_lock_not_taken_is_unfinished() {
        assert!(unfinished(
            Funding::AssetLockAddressTopUp,
            &Status::Broadcast
        ));
        assert!(unfinished(
            Funding::AssetLockAddressTopUp,
            &Status::InstantSendLocked
        ));
        assert!(!unfinished(
            Funding::AssetLockAddressTopUp,
            &Status::Consumed
        ));
        assert!(!unfinished(
            Funding::AssetLockAddressTopUp,
            &Status::RecoveredFromChain
        ));
        assert!(!unfinished(Funding::IdentityTopUp, &Status::Broadcast));
        assert!(!unfinished(
            Funding::AssetLockShieldedAddressTopUp,
            &Status::Broadcast
        ));
    }

    /// AC-T6: after a restore, a lock found on chain is offered to continue only when Platform
    /// proved it has not seen it; one it used, whole or in part, is not, and one not answered
    /// yet is not offered until it is. On substituted answers.
    #[test]
    fn a_recovered_lock_is_unfinished_only_when_platform_has_not_seen_it() {
        use crate::lock_status::Answer;
        use key_wallet::dashcore::hashes::Hash;
        let lock = |n: u8| RecoveredLock {
            txid: format!("{n:02x}").repeat(32),
            out_point: key_wallet::dashcore::OutPoint::new(
                key_wallet::dashcore::Txid::from_byte_array([n; 32]),
                0,
            ),
            amount: 10_012_000,
        };
        let locks = [lock(1), lock(2), lock(3)];
        let told = |answers: [Option<Answer>; 3]| {
            let locks = locks.clone();
            move |txid: &str| {
                let i = locks.iter().position(|l| l.txid == txid)?;
                answers[i]
            }
        };
        // Spent, partly used, not answered: nothing to continue.
        assert_eq!(
            recovered_unfinished(
                &locks,
                told([Some(Answer::Spent), Some(Answer::Partly { left: 5 }), None])
            ),
            None
        );
        // One Platform has not seen: that one.
        assert_eq!(
            recovered_unfinished(
                &locks,
                told([Some(Answer::Spent), None, Some(Answer::NotSeen)])
            ),
            Some(&locks[2])
        );
        assert_eq!(recovered_unfinished(&[], |_| Some(Answer::NotSeen)), None);
    }
}
