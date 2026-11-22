//! FEAT-0049 r2: the change addresses withdrawals from the pool go to, reserved until the payout
//! comes. key-wallet hands out a change address as "next unused" — not used until the network
//! shows a payment to it. A withdrawal from the pool is paid out minutes later (an asset unlock),
//! and meanwhile the same address would go to the next withdrawal or to the change of a Core
//! payment, linking the pool to that payment. A reserved address is skipped by both; the payout
//! makes it used. key-wallet's sweep of stale reservations covers receive addresses only, so a
//! change reservation stays until the payout or a release; a restart forgets it, and the core
//! holds it again from the pool's kept lines ([`hold`]).

use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use key_wallet::{AddressPool, KeySource, ManagedAccountType};

/// The BIP44 account whose change chain withdrawals use: the one «Receive» shows.
const ACCOUNT: u32 = 0;

/// The time a reservation is stamped with (seconds); 0 without a clock, which no sweep ages.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The change chain of account 0, and what derives its addresses.
fn change_pool(
    info: &mut ManagedWalletInfo,
    xpub: Option<key_wallet::bip32::ExtendedPubKey>,
    work: impl FnOnce(&mut AddressPool, &KeySource) -> Result<(String, bool), String>,
) -> Result<String, String> {
    let account = info
        .accounts_mut()
        .standard_bip44_accounts
        .get_mut(&ACCOUNT)
        .ok_or("No BIP44 account")?;
    let keys = account.keys_mut();
    let ManagedAccountType::Standard {
        internal_addresses, ..
    } = keys.managed_account_type_mut()
    else {
        return Err("Not a standard account".into());
    };
    let source = xpub.map_or(KeySource::NoKeySource, KeySource::Public);
    let (address, changed) = work(internal_addresses, &source)?;
    if changed {
        // A new address, or one now skipped: the watched set and the hand-outs know it.
        keys.bump_monitor_revision();
    }
    Ok(address)
}

/// The next unused change address, reserved at `now` (seconds): neither the next withdrawal nor
/// a Core payment's change takes it.
pub fn reserve(
    info: &mut ManagedWalletInfo,
    xpub: key_wallet::bip32::ExtendedPubKey,
    now: u64,
) -> Result<String, String> {
    change_pool(info, Some(xpub), |pool, source| {
        pool.next_unused_and_reserve(source, now)
            .map(|a| (a.to_string(), true))
            .map_err(|e| e.to_string())
    })
}

/// The withdrawals still on their way (`addresses`), reserved again at `now`: after a restart
/// (the reservation lives in memory) and to keep each one fresh. A used one is left as it is.
/// How many are reserved now.
pub fn hold(info: &mut ManagedWalletInfo, addresses: &[String], now: u64) -> usize {
    let mut held = 0;
    let _ = change_pool(info, None, |pool, _| {
        for entry in pool.addresses.values_mut() {
            if !entry.is_used() && addresses.contains(&entry.address.to_string()) {
                entry.state =
                    key_wallet::managed_account::address_pool::AddressState::Reserved { at: now };
                held += 1;
            }
        }
        Ok((String::new(), held > 0))
    });
    held
}

/// A withdrawal refused for sure: its address is free again.
pub fn release(info: &mut ManagedWalletInfo, address: &str) -> bool {
    let mut released = false;
    let _ = change_pool(info, None, |pool, _| {
        let index = pool
            .addresses
            .values()
            .find(|e| e.address.to_string() == address)
            .map(|e| e.index);
        released = index.is_some_and(|i| pool.release_reservation(i));
        Ok((String::new(), released))
    });
    released
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wallet() -> (ManagedWalletInfo, key_wallet::bip32::ExtendedPubKey) {
        let keys = crate::keys_of(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let info = ManagedWalletInfo::from_wallet(&keys, 0);
        let xpub = keys.accounts.standard_bip44_accounts[&ACCOUNT].account_xpub;
        (info, xpub)
    }

    /// The change a Core payment would get now, as key-wallet hands it out.
    fn payment_change(
        info: &mut ManagedWalletInfo,
        xpub: key_wallet::bip32::ExtendedPubKey,
    ) -> String {
        info.accounts_mut()
            .standard_bip44_accounts
            .get_mut(&ACCOUNT)
            .unwrap()
            .next_change_address(Some(&xpub), true)
            .unwrap()
            .to_string()
    }

    /// Review r2 Must: a Core payment made before the withdrawal arrives gets its change on
    /// another address; two withdrawals in a row get two addresses.
    #[test]
    fn a_reserved_change_address_goes_to_nobody_else() {
        let (mut info, xpub) = wallet();
        let first = reserve(&mut info, xpub, 1_000).unwrap();
        let change = payment_change(&mut info, xpub);
        assert_ne!(
            change, first,
            "a payment's change is not the withdrawal's address"
        );
        let second = reserve(&mut info, xpub, 1_001).unwrap();
        assert_ne!(second, first, "two withdrawals, two addresses");
        assert_ne!(
            payment_change(&mut info, xpub),
            second,
            "nor the second one's"
        );
    }

    /// Review r2 Must: after a restart the reservation is gone (a fresh wallet info) and comes
    /// back from the kept line; a refused withdrawal's address is free again.
    #[test]
    fn a_reservation_comes_back_after_a_restart_and_goes_on_a_refusal() {
        let (mut info, xpub) = wallet();
        let withdrawn = reserve(&mut info, xpub, 1_000).unwrap();
        // The restart: the same wallet, nothing reserved.
        let (mut again, _) = wallet();
        // The pool as the sync left it: the address is generated, still unused.
        payment_change(&mut again, xpub);
        assert_eq!(
            payment_change(&mut again, xpub),
            withdrawn,
            "free before the hold"
        );
        assert_eq!(hold(&mut again, std::slice::from_ref(&withdrawn), 2_000), 1);
        assert_ne!(payment_change(&mut again, xpub), withdrawn, "held again");
        assert!(release(&mut again, &withdrawn));
        assert_eq!(payment_change(&mut again, xpub), withdrawn, "released");
    }
}
