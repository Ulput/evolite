//! Claiming a token's distribution and buying a token from its issuer (FEAT-0019 T-2), past the
//! password gate. What refuses before the password is the screen's (no button without a proved
//! claim or price, the minimum, the credits); here only what cannot be done at all.

use evolite_api::tokens::TokenInfo;
use evolite_local::wallet_tokens::Tracked;

use super::identity::own_line;
use super::tokens::{make_due, token_label};
use crate::real::*;

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
        let unclear = self.unclear.clone();
        let refresh = self.refresher();
        // BUG-206: the identity the screen shows when the command comes: the task checks, pays
        // and signs as it even if the choice moves while it runs, and refuses if it is gone.
        let pinned = self.shown_now(id);
        self.answer_later("task:claim_token", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let (me, me_id) =
                    pinned_acting(&wallet, pinned)
                        .await
                        .ok_or(CoreError::NotAvailable(
                            tr!("The wallet has no identity").into(),
                        ))?;
                // BUG-205: an earlier write of this token not known to have gone through first;
                // the balance before, for the read that shows this one.
                super::tokens::earlier_settled(
                    &unclear, &journal, &chain, &wallet, &me, me_id, &info,
                )
                .await?;
                let held = super::tokens::held_now(&chain, &me, &token.token_id).await;
                // BUG-205 r1 M1: the identity's nonce with the token's contract, proved, before
                // signing: only its own writes of this contract move it.
                let n0 = chain.contract_nonce(&me_id, &token.contract_id).await.ok();
                let shows = held.map(|b| sent::TokenShows::BalanceAtLeast(b + 1));
                let before = chain.identity_credits(&wallet, &me_id, false).await;
                // TECH-after-send-outcome: never cut; without its answer, not known.
                let claim = {
                    let (chain, wallet, keys) = (chain.clone(), wallet.clone(), keys.clone());
                    let (contract, position) = (token.contract_id.clone(), token.position);
                    async move {
                        chain
                            .claim_token(&wallet, &keys, &me_id, &contract, position, scheduled)
                            .await
                    }
                };
                let outcome = sent::run(
                    claim,
                    sent::SEND_WAIT,
                    |e: &String| sent::refused_before(e),
                    || {
                        sent::token_state_if(
                            &chain,
                            &me_id,
                            &me,
                            &token.contract_id,
                            &token.token_id,
                            n0,
                            shows,
                        )
                    },
                    |state| n0.is_some_and(|n0| state.shows(n0, shows)),
                )
                .await;
                make_due(&facts);
                let label = token_label(&info);
                let decimals = info.decimals;
                let doc = match outcome.landed() {
                    Ok(sent::Landed::Answer(doc)) => doc,
                    Ok(sent::Landed::Read(read)) => {
                        // No document: the amount is the rise of the balance.
                        let units = read.saturating_sub(held.unwrap_or(read));
                        let kind = claimed_units(label, decimals, units);
                        let after = chain.identity_credits(&wallet, &me_id, true).await;
                        journal.add(own_line(kind, before, after, Some(me)));
                        refresh();
                        return Ok(());
                    }
                    Err(sent::NoAnswer::Refused(e)) => {
                        let token = label;
                        sent::refused_line(&chain, &wallet, me_id, before, token, me, &journal)
                            .await;
                        return Err(dashpay_failure(e));
                    }
                    Err(sent::NoAnswer::Unclear(late)) => {
                        // The amount is the claim document's: the line is made from the answer.
                        let pending = sent::Pending::new(
                            me.clone(),
                            sent::token_watch(&token.token_id, &token.contract_id, n0, shows),
                            None,
                            before,
                            Some(me),
                        );
                        return Err(unclear.not_known(
                            pending,
                            late,
                            journal,
                            move |doc, p| async move {
                                let kind = claimed(label, decimals, &doc);
                                let mut line =
                                    sent::line_after(chain, wallet, me_id, Some(kind), p).await;
                                line.key = Some(format!("tokx:{}", doc.id));
                                line
                            },
                        ));
                    }
                };
                let after = chain.identity_credits(&wallet, &me_id, true).await;
                let mut line = own_line(claimed(label, decimals, &doc), before, after, Some(me));
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
        let unclear = self.unclear.clone();
        let refresh = self.refresher();
        // BUG-206: the identity the screen shows when the command comes: the task checks, pays
        // and signs as it even if the choice moves while it runs, and refuses if it is gone.
        let pinned = self.shown_now(id);
        self.answer_later("task:buy_token", async move {
            let result = async {
                let wallet = loaded(&chain, &id).await?;
                let (me, me_id) =
                    pinned_acting(&wallet, pinned)
                        .await
                        .ok_or(CoreError::NotAvailable(
                            tr!("The wallet has no identity").into(),
                        ))?;
                // BUG-205: an earlier write of this token not known to have gone through first;
                // the balance before, for the read that shows this one.
                super::tokens::earlier_settled(
                    &unclear, &journal, &chain, &wallet, &me, me_id, &info,
                )
                .await?;
                let held = super::tokens::held_now(&chain, &me, &token.token_id).await;
                // BUG-205 r1 M1: the identity's nonce with the token's contract, proved, before
                // signing: only its own writes of this contract move it.
                let n0 = chain.contract_nonce(&me_id, &token.contract_id).await.ok();
                let shows = held.map(|b| sent::TokenShows::BalanceAtLeast(b + count));
                let before = chain.identity_credits(&wallet, &me_id, false).await;
                // TECH-after-send-outcome: never cut; without its answer, not known.
                let buy = {
                    let (chain, wallet, keys) = (chain.clone(), wallet.clone(), keys.clone());
                    let (contract, position) = (token.contract_id.clone(), token.position);
                    async move {
                        chain
                            .buy_token(&wallet, &keys, &me_id, &contract, position, count, agreed)
                            .await
                    }
                };
                let outcome = sent::run(
                    buy,
                    sent::SEND_WAIT,
                    |e: &chain::BuyRefused| match e {
                        chain::BuyRefused::PriceGrew => true,
                        chain::BuyRefused::Other(e) => sent::refused_before(e),
                    },
                    || {
                        sent::token_state_if(
                            &chain,
                            &me_id,
                            &me,
                            &token.contract_id,
                            &token.token_id,
                            n0,
                            shows,
                        )
                    },
                    |state| n0.is_some_and(|n0| state.shows(n0, shows)),
                )
                .await;
                // Either way the balance and the price are read again: the row shows the new
                // price after a refusal (P2), the new balance after a purchase.
                make_due(&facts);
                let label = token_label(&info);
                let decimals = info.decimals;
                let bought = match outcome.landed() {
                    Ok(sent::Landed::Answer(bought)) => bought,
                    // The balance shows the purchase: the count asked for.
                    Ok(sent::Landed::Read(read)) => chain::Bought::Balance(read),
                    Err(sent::NoAnswer::Refused(chain::BuyRefused::PriceGrew)) => {
                        refresh();
                        return Err(CoreError::TokenPriceGrew);
                    }
                    Err(sent::NoAnswer::Refused(chain::BuyRefused::Other(e))) => {
                        sent::refused_line(&chain, &wallet, me_id, before, label, me, &journal)
                            .await;
                        return Err(dashpay_failure(e));
                    }
                    Err(sent::NoAnswer::Unclear(late)) => {
                        // The kind as the read would show it: the count asked for.
                        let kind =
                            bought_line(label.clone(), decimals, count, chain::Bought::Balance(0))
                                .1;
                        let pending = sent::Pending::new(
                            me.clone(),
                            sent::token_watch(&token.token_id, &token.contract_id, n0, shows),
                            Some(kind),
                            before,
                            Some(me),
                        );
                        return Err(unclear.not_known(
                            pending,
                            late,
                            journal,
                            move |bought, p| async move {
                                let (key, kind) = bought_line(label, decimals, count, bought);
                                let mut line =
                                    sent::line_after(chain, wallet, me_id, Some(kind), p).await;
                                line.key = key;
                                line
                            },
                        ));
                    }
                };
                let after = chain.identity_credits(&wallet, &me_id, true).await;
                let (key, kind) = bought_line(label, decimals, count, bought);
                let mut line = own_line(kind, before, after, Some(me));
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

/// A claim's History line kind, from its proved document.
fn claimed(token: String, decimals: u8, doc: &chain::ClaimDoc) -> journal::Kind {
    claimed_units(token, decimals, doc.amount)
}

/// A claim's History line kind of `units` claimed.
pub(in crate::real) fn claimed_units(token: String, decimals: u8, units: u64) -> journal::Kind {
    journal::Kind::TokenClaimed {
        token,
        amount: evolite_api::tokens::amount_text(units, decimals),
        units,
    }
}

/// A purchase's History line kind and key: what its proved answer says was bought.
fn bought_line(
    token: String,
    decimals: u8,
    count: u64,
    bought: chain::Bought,
) -> (Option<String>, journal::Kind) {
    let (key, units) = match bought {
        chain::Bought::Document { id, amount } => (Some(format!("tokx:{id}")), amount),
        chain::Bought::Balance(_) => (None, count),
    };
    let kind = journal::Kind::TokenBought {
        token,
        amount: evolite_api::tokens::amount_text(units, decimals),
        units,
    };
    (key, kind)
}
