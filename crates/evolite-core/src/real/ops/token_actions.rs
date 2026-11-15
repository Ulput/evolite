//! Claiming a token's distribution and buying a token from its issuer (FEAT-0019 T-2), past the
//! password gate. What refuses before the password is the screen's (no button without a proved
//! claim or price, the minimum, the credits); here only what cannot be done at all.

use evolite_api::tokens::TokenInfo;
use evolite_local::wallet_tokens::Tracked;

use super::identity::own_line;
use super::tokens::{make_due, token_label};
use crate::real::*;

/// How long a claim or a purchase may take before the answer says it timed out; it may still
/// land, and the next read of the balances shows it.
const ACT_WAIT: Duration = Duration::from_secs(120);

impl Real {
    /// The followed token, its kept contract read, and the open keys; or why not.
    fn token_to_act(
        &self,
        token_id: &str,
    ) -> Result<(Tracked, TokenInfo, WalletId, key_wallet::Wallet), CoreError> {
        let Some(file) = self.tokens.file.clone() else {
            return Err(CoreError::NotAvailable(
                tr!("Tokens are not on this network").into(),
            ));
        };
        let token = file
            .get(token_id)
            .ok_or_else(|| CoreError::Failed("the token is not in the list".into()))?;
        let info = self
            .tokens
            .infos_of(&file)
            .get(token_id)
            .cloned()
            .ok_or_else(|| CoreError::Failed("the token's contract is not read yet".into()))?;
        if !self.chain.platform_ready() {
            return Err(CoreError::NotSynced);
        }
        let (id, keys) = self
            .unlocked
            .as_ref()
            .and_then(|open| Some((open.id, open.keys.clone()?)))
            .ok_or_else(|| CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        Ok((token, info, id, keys))
    }

    /// Claims the shown identity's distribution of `token_id` (Spend::ClaimToken, T-2.3): the
    /// History line takes its amount and key from the proved `claim` document; the balance is
    /// read again. `Done { what: DoneWhat::ClaimToken }`.
    pub(in crate::real) fn claim_token(&self, token_id: String, scheduled: bool) -> Event {
        let (token, info, id, keys) = match self.token_to_act(&token_id) {
            Ok(found) => found,
            Err(e) => {
                return Event::Done {
                    what: DoneWhat::ClaimToken,
                    result: Err(e),
                };
            }
        };
        let (chain, facts, journal) = (
            self.chain.clone(),
            self.tokens.facts_handle(),
            self.journal.clone(),
        );
        let refresh = self.refresher();
        self.answer_later("task:claim_token", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let me = acting_identity(&wallet)
                    .await
                    .ok_or(CoreError::NotAvailable(
                        tr!("The wallet has no identity").into(),
                    ))?;
                let before = chain.first_identity_credits(&wallet, false).await;
                let claim = chain.claim_token(
                    &wallet,
                    &keys,
                    &token.contract_id,
                    token.position,
                    scheduled,
                );
                let doc = tokio::time::timeout(ACT_WAIT, claim)
                    .await
                    .map_err(|_| CoreError::Failed("timed out".into()))?
                    .map_err(dashpay_failure)?;
                make_due(&facts);
                let after = chain.first_identity_credits(&wallet, true).await;
                let mut line = own_line(
                    journal::Kind::TokenClaimed {
                        token: token_label(&info),
                        amount: evolite_api::tokens::amount_text(doc.amount, info.decimals),
                        units: doc.amount,
                    },
                    before,
                    after,
                    Some(me),
                );
                // Q4: keyed by its document at once; the same document read later is this line.
                line.key = Some(format!("tokx:{}", doc.id));
                journal.add(line);
                refresh();
                Ok(())
            }
            .await;
            Event::Done {
                what: DoneWhat::ClaimToken,
                result,
            }
        });
        Event::Progress {
            what: "claim_token",
            fraction: 0.0,
        }
    }

    /// Buys `count` of `token_id` for at most `agreed` credits (Spend::BuyToken, T-2.5). A price
    /// that grew is refused with nothing taken, and the price is read again for the screen.
    /// `Done { what: DoneWhat::BuyToken }`.
    pub(in crate::real) fn buy_token(&self, token_id: String, count: u64, agreed: u64) -> Event {
        let refused = |e| Event::Done {
            what: DoneWhat::BuyToken,
            result: Err(e),
        };
        if count == 0 || agreed == 0 {
            return refused(CoreError::BadAmount);
        }
        let (token, info, id, keys) = match self.token_to_act(&token_id) {
            Ok(found) => found,
            Err(e) => return refused(e),
        };
        let (chain, facts, journal) = (
            self.chain.clone(),
            self.tokens.facts_handle(),
            self.journal.clone(),
        );
        let refresh = self.refresher();
        self.answer_later("task:buy_token", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let me = acting_identity(&wallet)
                    .await
                    .ok_or(CoreError::NotAvailable(
                        tr!("The wallet has no identity").into(),
                    ))?;
                let before = chain.first_identity_credits(&wallet, false).await;
                let buy = chain.buy_token(
                    &wallet,
                    &keys,
                    &token.contract_id,
                    token.position,
                    count,
                    agreed,
                );
                let bought = tokio::time::timeout(ACT_WAIT, buy)
                    .await
                    .map_err(|_| CoreError::Failed("timed out".into()))?;
                // Either way the balance and the price are read again: the row shows the new
                // price after a refusal (P2), the new balance after a purchase.
                make_due(&facts);
                let bought = match bought {
                    Ok(bought) => bought,
                    Err(chain::BuyRefused::PriceGrew) => {
                        refresh();
                        return Err(CoreError::TokenPriceGrew);
                    }
                    Err(chain::BuyRefused::Other(e)) => return Err(dashpay_failure(e)),
                };
                let after = chain.first_identity_credits(&wallet, true).await;
                let (key, units) = match bought {
                    chain::Bought::Document { id, amount } => (Some(format!("tokx:{id}")), amount),
                    chain::Bought::Balance(_) => (None, count),
                };
                let mut line = own_line(
                    journal::Kind::TokenBought {
                        token: token_label(&info),
                        amount: evolite_api::tokens::amount_text(units, info.decimals),
                        units,
                    },
                    before,
                    after,
                    Some(me),
                );
                line.key = key;
                journal.add(line);
                refresh();
                Ok(())
            }
            .await;
            Event::Done {
                what: DoneWhat::BuyToken,
                result,
            }
        });
        Event::Progress {
            what: "buy_token",
            fraction: 0.0,
        }
    }
}
