//! FEAT-0042: the account's public key — BIP44 account 0 of Core — after the wallet's password,
//! every time and counted with the unlock's wrong tries (SEC-11). Behind the advanced switch,
//! refused before the password when it is off (AC-T-X1). The key is the one platform-wallet keeps
//! as public data; before the chain has loaded the wallet, the open keys' account gives the same
//! public key (it is derived when the wallet opens, nothing private is read for it). The journal
//! has the command's name and the outcome only (R-5).

use crate::real::*;

/// The account's path on this network (R-12).
pub(in crate::real) fn account_path() -> &'static str {
    match crate::network::current() {
        evolite_api::Network::Mainnet => "m/44'/5'/0'",
        _ => "m/44'/1'/0'",
    }
}

impl Real {
    /// `Command::ExportXpub`.
    pub(in crate::real) async fn export_xpub(
        &mut self,
        password: &Password,
    ) -> Result<evolite_api::Xpub, CoreError> {
        if !self.advanced_features {
            return Err(CoreError::NotAvailable(evolite_common::ADVANCED_OFF.into()));
        }
        let id = self.unlocked.as_ref().ok_or_else(locked)?.id;
        self.checked_vault(password).await?;
        let key = match self.chain.account_xpub(&id).await {
            Some(key) => key,
            None => self
                .unlocked
                .as_ref()
                .and_then(|u| u.keys.as_ref())
                .and_then(|k| k.accounts.standard_bip44_accounts.get(&0))
                .map(|a| a.account_xpub.to_string())
                .ok_or(CoreError::NotSynced)?,
        };
        Ok(evolite_api::Xpub::new(key, account_path().into()))
    }
}
