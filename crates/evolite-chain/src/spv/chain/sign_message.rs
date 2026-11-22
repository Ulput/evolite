//! FEAT-0044: signing a message with an address of this wallet — platform-wallet's
//! `CoreWallet::sign_message` (dash-qt's format, compressed key, base64), with the open wallet's
//! keys. Only an address of BIP44 account 0, receiving or change (R-11, the customer's answer to
//! analyst-162): a mixed address signed would tie the person to the mixing, a contact's is one
//! contact's.

use super::Chain;

/// Why a message was not signed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignError {
    /// Not an address at all, or not of this network.
    NotAnAddress,
    /// An address of this wallet, but not BIP44 account 0 (CoinJoin, a contact's).
    NotCoreAccount,
    /// Not an address of this wallet.
    NotOurs,
    /// platform-wallet refused or failed: its words, for the journal's kind only.
    Failed(String),
}

impl Chain {
    /// R-11, before the password: `address` is of this network and of BIP44 account 0 of
    /// `wallet`, receiving or change. The address as the network writes it.
    pub async fn signing_address(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        address: &str,
    ) -> Result<String, SignError> {
        use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
        use std::str::FromStr;
        let parsed = key_wallet::dashcore::Address::<
            key_wallet::dashcore::address::NetworkUnchecked,
        >::from_str(address.trim())
        .ok()
        .and_then(|a| a.require_network(crate::network::dash()).ok())
        .ok_or(SignError::NotAnAddress)?;
        let state = wallet.state().await;
        let accounts = state.core_wallet.accounts();
        let mut ours = false;
        for account in accounts.all_accounts() {
            if !account.contains_address(&parsed) {
                continue;
            }
            ours = true;
            if matches!(
                account.managed_account_type(),
                key_wallet::ManagedAccountType::Standard {
                    index: 0,
                    standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
                    ..
                }
            ) {
                return Ok(parsed.to_string());
            }
        }
        Err(if ours {
            SignError::NotCoreAccount
        } else {
            SignError::NotOurs
        })
    }

    /// Signs `message` (its bytes exactly) with `address` of `wallet` — checked again here.
    pub async fn sign_message(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        keys: &key_wallet::Wallet,
        address: &str,
        message: &str,
    ) -> Result<String, SignError> {
        let address = self.signing_address(wallet, address).await?;
        let signer = crate::platform::identity::AssetLockSigner(keys.clone());
        wallet
            .core()
            .sign_message(&address, message, &signer)
            .await
            .map_err(|e| SignError::Failed(e.to_string()))
    }
}
