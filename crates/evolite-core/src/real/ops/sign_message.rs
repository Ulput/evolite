//! FEAT-0044: «Sign a message» with an address of this wallet. Behind the advanced switch; the
//! address is checked before the password (R-11, AC-T-S1); the password every time, counted with
//! the unlock's (R-13); the signature is platform-wallet's, made with the open keys. The journal
//! has the command's name and the outcome only (R-7): no address, no text, no signature.

use crate::real::*;
use evolite_chain::chain::sign_message::SignError;

/// R-9: the most characters a message may have.
pub(in crate::real) const MESSAGE_MAX: usize = 10_000;

impl Real {
    /// `Command::SignMessage`.
    pub(in crate::real) async fn sign_message(
        &mut self,
        address: &str,
        message: &str,
        password: &Password,
    ) -> Result<String, CoreError> {
        if !self.advanced_features {
            return Err(CoreError::NotAvailable(evolite_common::ADVANCED_OFF.into()));
        }
        let id = self.unlocked.as_ref().ok_or_else(locked)?.id;
        if message.chars().count() > MESSAGE_MAX {
            return Err(CoreError::NotAvailable(
                tr!("At most 10,000 characters").into(),
            ));
        }
        let wallet = loaded(&self.chain, &id).await?;
        let words = |e: SignError| match e {
            SignError::NotAnAddress | SignError::NotOurs => {
                CoreError::NotAvailable(tr!("This is not an address of this wallet").into())
            }
            SignError::NotCoreAccount => CoreError::NotAvailable(
                tr!("Only a Core address for receiving or change can sign").into(),
            ),
            SignError::Failed(why) => CoreError::Failed(why),
        };
        self.chain
            .signing_address(&wallet, address)
            .await
            .map_err(words)?;
        self.checked_vault(password).await?;
        let keys = self
            .unlocked
            .as_ref()
            .and_then(|u| u.keys.as_ref())
            .ok_or_else(locked)?
            .clone();
        self.chain
            .sign_message(&wallet, &keys, address, message)
            .await
            .map_err(words)
    }
}
