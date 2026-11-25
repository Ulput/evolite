//! The commands: which operation each one is, and the answer it gets.

use super::ops::series_ops::Start;
use super::*;

/// What a spend that takes the identity says while another holds it: the words its own start
/// says (TECH-busy-guard), now said before the password.
fn busy_refusal(op: &Spend) -> CoreError {
    match op {
        Spend::CreateIdentity { .. }
        | Spend::CreateIdentityFromPool { .. }
        | Spend::CreateIdentityFromMixed { .. }
        | Spend::ResumeIdentity => {
            CoreError::NotAvailable(tr!("An identity is already being made").into())
        }
        Spend::TopUpIdentity { .. }
        | Spend::TopUpFromMixed { .. }
        | Spend::ResumeTopUp
        | Spend::AddName { .. }
        | Spend::FundAddress { .. }
        | Spend::ResumeFundAddress => {
            CoreError::NotAvailable(tr!("An identity is already being made or topped up").into())
        }
        _ => ops::identity_keys::identity_busy(),
    }
}

impl Real {
    /// A spend past [`Real::spend_gate`], answered as each operation always was.
    async fn spend(&mut self, op: Spend) -> (Option<Event>, bool) {
        let event = match op {
            Spend::Send {
                id,
                from,
                to,
                amount,
                note,
                rate,
                coins,
                fee_rate,
                fee_shown,
            } => {
                // FEAT-0040 (AC-T-F5): with the advanced features off, the normal rate, whatever
                // the form sent.
                let fee_rate = fee_rate.filter(|_| self.advanced_features);
                match self
                    .send(
                        id,
                        from,
                        &to,
                        amount,
                        (note, rate, coins),
                        (fee_rate, fee_shown),
                    )
                    .await
                {
                    // The answer comes from the broadcast task.
                    Ok(()) => return (None, true),
                    Err(e) => Event::Sent { id, result: Err(e) },
                }
            }
            Spend::SendMixed {
                id,
                to,
                amount,
                note,
                rate,
                accept_excess,
            } => match self
                .send_mixed(id, &to, amount, (note, rate, accept_excess))
                .await
            {
                Ok(()) => return (None, true),
                Err(e) => Event::Sent { id, result: Err(e) },
            },
            Spend::CreateIdentity { name, top_up } => {
                match self.create_identity(name.as_deref(), top_up).await {
                    // The answer comes from the background task.
                    Ok(()) => return (None, true),
                    Err(e) => Event::Done {
                        what: DoneWhat::Identity,
                        result: Err(e),
                        ask: None,
                    },
                }
            }
            // FEAT-0048: privately, from the pool; progress as the pool's moves report it.
            Spend::CreateIdentityFromPool { name, exit } => {
                match self.create_identity_from_pool(name.as_deref(), exit).await {
                    Ok(()) => Event::Progress {
                        what: "identity-from-pool",
                        fraction: 0.0,
                    },
                    Err(e) => Event::Done {
                        what: DoneWhat::Identity,
                        result: Err(e),
                        ask: None,
                    },
                }
            }
            // FEAT-0048 UC-4: a refused creation's money back into the pool.
            // FEAT-0048 step 2: fully mixed coins to Platform.
            Spend::CreateIdentityFromMixed {
                name,
                amount,
                accept_excess,
            } => match self
                .create_identity_from_mixed(name.as_deref(), amount, accept_excess)
                .await
            {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Identity,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::TopUpFromMixed {
                amount,
                accept_excess,
            } => match self.top_up_from_mixed(amount, accept_excess).await {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::TopUp,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::ShieldFromMixed {
                amount,
                accept_excess,
            } => match self.shield_from_mixed(amount, accept_excess).await {
                Ok(()) => Event::Progress {
                    what: "shield-from-mixed",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: DoneWhat::Shield,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::ShieldRefund => match self.shield_refund() {
                Ok(()) => Event::Progress {
                    what: "shield-refund",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: DoneWhat::Shield,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::ResumeIdentity => match self.resume_identity().await {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Identity,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::Shield { from, amount } => match self.shield(from, amount) {
                Ok(()) => Event::Progress {
                    what: "shield",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: DoneWhat::Shield,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::SendToken {
                token_id,
                to,
                amount,
            } => match self.send_token(token_id, to, amount) {
                // The answer comes from the background task.
                Event::Progress { .. } => return (None, true),
                refused => refused,
            },
            Spend::ClaimToken {
                token_id,
                scheduled,
            } => match self.claim_token(token_id, scheduled) {
                Event::Progress { .. } => return (None, true),
                refused => refused,
            },
            Spend::BuyToken {
                token_id,
                count,
                agreed,
            } => match self.buy_token(token_id, count, agreed) {
                Event::Progress { .. } => return (None, true),
                refused => refused,
            },
            Spend::IssuerAction { token_id, op } => match self.issuer_action(token_id, op) {
                Event::Progress { .. } => return (None, true),
                refused => refused,
            },
            // FEAT-0038 module V1: a contract of the wallet's own.
            Spend::RegisterContract {
                identity,
                draft,
                fee_max,
            } => match self.register_contract(identity, draft, fee_max) {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Contract,
                    result: Err(e),
                    ask: None,
                },
            },
            // FEAT-0038: a «Workshop» series, new or continued.
            Spend::NftBatch {
                target,
                kind,
                fee_max,
            } => match self.run_series(Start::New(target, kind), fee_max) {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Series,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::ContinueSeries { id, fee_max } => {
                match self.run_series(Start::Continue(id), fee_max) {
                    Ok(()) => return (None, true),
                    Err(e) => Event::Done {
                        what: DoneWhat::Series,
                        result: Err(e),
                        ask: None,
                    },
                }
            }
            Spend::CreateModel {
                identity,
                contract,
                model,
                mint,
                fee_max,
            } => match self.run_series(
                Start::Model {
                    identity,
                    contract,
                    model,
                    mint,
                },
                fee_max,
            ) {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Series,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::GiftNft { .. } => Event::Done {
                what: DoneWhat::Gift,
                result: Err(CoreError::NotAvailable(NO_GIFTS.into())),
                ask: None,
            },
            Spend::AddContact { name } => match self.add_contact(&name) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: DoneWhat::Contact,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::AcceptContact { identity_id } => {
                match self.contact_job(&identity_id, ContactJob::Accept) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::Done {
                        what: DoneWhat::Contact,
                        result: Err(e),
                        ask: None,
                    },
                }
            }
            Spend::SaveProfile {
                display,
                about,
                avatar_url,
            } => match self.save_profile(display, about, avatar_url) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: DoneWhat::Profile,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::TopUpIdentity { amount } => match self.top_up_identity(amount).await {
                // The answer comes from the background task.
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::TopUp,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::ResumeTopUp => match self.resume_top_up() {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::TopUp,
                    result: Err(e),
                    ask: None,
                },
            },
            // FEAT-0015 stage 2: the answer comes from the background task.
            Spend::FundAddress { amount } => match self.fund_address(amount).await {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::FundAddress,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::ResumeFundAddress => match self.resume_fund_address().await {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::FundAddress,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::Unshield { amount } => match self.unshield(amount) {
                Ok(()) => Event::Progress {
                    what: "unshield",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: DoneWhat::Unshield,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::UnshieldToCore { amount } => match self.unshield_to_core(amount) {
                Ok(()) => Event::Progress {
                    what: "unshield",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: DoneWhat::Unshield,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::TopUpFromPool { amount } => match self.top_up_from_pool(amount) {
                Ok(()) => Event::Progress {
                    what: "pool-top-up",
                    fraction: 0.0,
                },
                Err(e) => Event::Done {
                    what: DoneWhat::PoolTopUp,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::AddName {
                name,
                contest_fund,
                link,
            } => match self.add_name(&name, contest_fund, link).await {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Name,
                    result: Err(e),
                    ask: None,
                },
            },
            // FEAT-0021 АРХ-6: the claim's link alone (AC-T12).
            Spend::SetNameLink { label, url } => match self.set_name_link(label, url).await {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Name,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::UpdateKeys {
                identity,
                add,
                disable,
            } => match self.update_keys(identity, add, disable) {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Keys,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::ResendRequests { identity, contact } => {
                match self.resend_requests(identity, contact) {
                    Ok(()) => return (None, true),
                    Err(e) => Event::Done {
                        what: DoneWhat::Resend,
                        result: Err(e),
                        ask: None,
                    },
                }
            }
            Spend::DisconnectApp { id } => match self.disconnect_app(&id) {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Disconnect,
                    result: Err(e),
                    ask: None,
                },
            },
            Spend::ApproveApp { id, grant } => match self.answer_app(id, true, grant) {
                // The answer comes from the background task.
                Ok(true) => return (None, true),
                Ok(false) => Event::Done {
                    what: DoneWhat::App,
                    result: Ok(()),
                    ask: None,
                },
                Err(e) => Event::Done {
                    what: DoneWhat::App,
                    result: Err(e),
                    ask: None,
                },
            },
        };
        (Some(event), true)
    }

    /// A spend through the one gate (R-27): refused, it is answered as the operation's own
    /// failure, and nothing of it was checked, locked or signed.
    async fn handle_spend(
        &mut self,
        op: Spend,
        password: Option<Password>,
        large_confirmed: bool,
    ) -> (Option<Event>, bool) {
        // FEAT-0004 FR-7 (AC-T5): on mainnet, more than the threshold without the tick is
        // refused first — no password looked at, nothing counted, nothing locked.
        if !large_confirmed && self.large_unconfirmed(&op).await {
            return (Some(op.refused(CoreError::LargeSpendNotConfirmed)), true);
        }
        // One change of the identity at a time (the review's Should 1): every spend that
        // takes it is refused before the password while another holds it
        // (TECH-busy-guard), in the words its own start would have said.
        if op.takes_identity() && self.identity_busy.is_busy() {
            return (Some(op.refused(busy_refusal(&op))), true);
        }
        // An asset lock the coins cannot make (SPV Б2, item 3) is refused before the
        // password is even looked at: no password typed in vain.
        if let Err(why) = self.lock_coins_before_password(&op).await {
            return (Some(op.refused(why)), true);
        }
        // A key change the identity cannot take is refused before the password
        // (FEAT-0020 AC-T2).
        if let Spend::UpdateKeys {
            identity,
            add,
            disable,
        } = &op
            && let Some(why) = self.key_update_refused(identity, add, disable).await
        {
            return (Some(op.refused(why)), true);
        }
        // FEAT-0040 (AC-T-F3): a rate out of bounds is refused before the password.
        if let Spend::Send {
            fee_rate: Some(rate),
            ..
        } = &op
            && evolite_api::fee_choice::refused(*rate)
        {
            return (Some(op.refused(fee_rate_refusal())), true);
        }
        if let Err(why) = self
            .spend_gate(password.as_ref(), op.asks_password_always())
            .await
        {
            return (Some(op.refused(why)), true);
        }
        self.prices.begin(&op, now());
        let (event, changed) = self.spend(op).await;
        if let Some(event) = &event {
            self.prices.finish(event);
        }
        (event, changed)
    }

    pub(super) async fn handle(&mut self, command: Command) -> (Option<Event>, bool) {
        let event = match command {
            Command::NewPhrase { words } => return self.new_phrase(words),
            Command::UsePhrase { phrase, dashpay } => {
                return self.use_phrase(phrase, dashpay).await;
            }
            Command::CreateWallet { .. } if self.leftovers => Event::Done {
                what: DoneWhat::Create,
                result: Err(CoreError::NotAvailable(LEFTOVERS.into())),
                ask: None,
            },
            Command::CreateWallet {
                password,
                passphrase,
            } => Event::Done {
                what: DoneWhat::Create,
                result: self.create(&password, passphrase.as_ref()).await,
                ask: None,
            },
            Command::Unlock { password } => Event::Done {
                what: DoneWhat::Unlock,
                result: self.unlock_counted(&password).await,
                ask: None,
            },
            Command::Lock => Event::Done {
                what: DoneWhat::Lock,
                result: self.lock().await,
                ask: None,
            },
            Command::AutoLock { after_secs } => {
                self.auto_lock = after_secs.map(Duration::from_secs);
                self.last_touch = tokio::time::Instant::now();
                return (None, false);
            }
            Command::Touch => {
                self.last_touch = tokio::time::Instant::now();
                return (None, false);
            }
            Command::CoreNode { address } => return self.core_node(address).await,
            Command::FiatRate { currency } => {
                self.fiat_rate(currency);
                return (None, true);
            }
            Command::CheckUpdate { on } => {
                self.check_update(on);
                return (None, true);
            }
            Command::ReceiveAddress { fresh } => {
                self.fresh_receive = fresh;
                // The next snapshot takes the address again.
                self.receive_address = None;
                return (None, true);
            }
            Command::ChangePassword { old, new } => Event::Done {
                what: DoneWhat::Password,
                result: self.change_password(&old, &new).await,
                ask: None,
            },
            Command::RevealPhrase { password, wallet } => {
                // Counted as the unlock is, for the wallet whose phrase it is (BUG-203): no
                // guessing another wallet's password from an open one without the pause. Only
                // from an open wallet (r1 note).
                let opened = match wallet.filter(|id| *id != self.wallet) {
                    _ if self.unlocked.is_none() => Err(locked()),
                    Some(id) => self.open_other_counted(&id, &password).await,
                    None => self.open_vault_counted(&password).await,
                };
                // FEAT-0041 R-3: with its BIP-39 passphrase (empty: none, and no line for it).
                let result = opened.map(|(_, secret)| {
                    Phrase::with_passphrase(
                        secret
                            .phrase
                            .split_whitespace()
                            .map(str::to_owned)
                            .collect(),
                        passphrase::Passphrase::new(secret.passphrase.to_string()),
                    )
                });
                Event::PhraseRevealed(result)
            }
            Command::SignMessage {
                address,
                message,
                password,
            } => Event::MessageSigned(self.sign_message(&address, &message, &password).await),
            Command::ExportXpub { password } => Event::Xpub(self.export_xpub(&password).await),
            Command::RevealIdentityKey {
                identity,
                key_id,
                password,
            } => Event::IdentityKeyRevealed(
                self.reveal_identity_key(&identity, key_id, &password).await,
            ),
            Command::ResolveName { name } => return self.resolve_name(name).await,
            Command::SuggestNames { prefix, seq } => {
                return (self.suggest_names(prefix, seq), false);
            }
            Command::AdvancedFeatures { on } => {
                // BUG-252, FEAT-0041 r1: a locked wallet keeps it off; with none on this
                // computer, or one added from an open wallet, there is nothing locked.
                let on = on && self.advanced_allowed();
                if on != self.advanced_features {
                    crate::diag::record(crate::diag::Record::AdvancedFeatures { on });
                }
                self.advanced_features = on;
                return (None, true);
            }
            Command::StrangerPictures { on } => {
                self.stranger_pictures(on);
                return (None, false);
            }
            Command::CheckName { label: asked } => return self.check_name(asked).await,
            // FEAT-0021 Should 1: «×» on Overview's notice, kept until the result; no answer.
            Command::CloseNameNotice { identity, label } => {
                let _ = self
                    .name_requests
                    .update(&identity, &label, |r| r.close_notice());
                return (None, true);
            }
            Command::HideNameRequest { identity, label } => Event::Done {
                what: DoneWhat::Name,
                result: self.hide_name_request(&identity, &label),
                ask: None,
            },
            Command::OpenAppLink { text } => match self.open_app_link(&text) {
                // The decoded request comes from the background task.
                Ok(()) => return (None, false),
                Err(e) => Event::AppRequest(Err(e)),
            },
            // NFT pictures as their documents store them (packed), or profile pictures: a
            // plain image file, never packed.
            Command::DrawArt { art, side, urgent } => return self.draw_art(art, side, urgent),
            // FEAT-0038 R-21: «Make a smaller copy», in the renderer's sandbox.
            Command::FitPicture { image, room } => match &self.artist {
                Some(artist) => {
                    artist.fit(image.into(), room);
                    return (None, false);
                }
                None => Event::PictureFitted {
                    result: Err(CoreError::NotAvailable(tr!("No picture").into())),
                },
            },
            Command::HideRequest { identity_id } => {
                match self.contact_job(&identity_id, ContactJob::Hide) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::Done {
                        what: DoneWhat::Contact,
                        result: Err(e),
                        ask: None,
                    },
                }
            }
            Command::ListCoins { ask } => Event::Coins {
                ask,
                result: self.list_coins().await,
            },
            Command::PreviewMixed { ask, to, amount } => Event::MixedPreview {
                ask,
                result: self.preview_mixed(&to, amount).await,
            },
            // FEAT-0048 step 2: the same coins whatever they go to; `to` is for the screens.
            Command::PreviewMixedLock { ask, amount, .. } => Event::MixedLockPreview {
                ask,
                result: self.preview_mixed_lock(amount).await,
            },
            Command::PreviewPayment { to } => match self.preview_payment(to.clone()) {
                Ok(()) => return (None, false),
                Err(e) => Event::PaymentPreview { to, result: Err(e) },
            },
            Command::ForgetApp { id } => match self.forget_app(&id) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: DoneWhat::ForgetApp,
                    result: Err(e),
                    ask: None,
                },
            },
            Command::ReadAppBudgets { id } => match self.read_app_budgets(&id) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: DoneWhat::AppBudgets,
                    result: Err(e),
                    ask: None,
                },
            },
            // Every spend passes the one gate first (R-27): refused, it is answered as the
            // operation's own failure, and nothing of it was checked, locked or signed.
            Command::Spend {
                op,
                password,
                ask,
                large_confirmed,
            } => {
                // TECH-spend-flow: the answer carries the window's number back, whether it comes
                // now or from a job started for it ([`Self::answer_later`]).
                self.spend_ask = Some(ask);
                let (event, changed) = self.handle_spend(op, password, large_confirmed).await;
                self.spend_ask = None;
                return (event.map(|e| e.answering(ask)), changed);
            }
            Command::SetLargeSpend { duffs, password } => Event::Done {
                what: DoneWhat::LargeSpend,
                result: self.set_large_spend(duffs, password.as_ref()).await,
                ask: None,
            },
            Command::SetPasswordEachSend { on, password } => Event::Done {
                what: DoneWhat::PasswordEachSend,
                result: self.set_password_each_send(on, password.as_ref()).await,
                ask: None,
            },
            // Turning a request down spends nothing: no password.
            Command::DeclineApp { id } => match self.answer_app(id, false, None) {
                Ok(_) => Event::Done {
                    what: DoneWhat::App,
                    result: Ok(()),
                    ask: None,
                },
                Err(e) => Event::Done {
                    what: DoneWhat::App,
                    result: Err(e),
                    ask: None,
                },
            },
            Command::SaveAddress { address, label } => return self.save_address(address, label),
            Command::ForgetAddress { address } => {
                let result = self
                    .address_book
                    .forget(address.trim())
                    .map_err(CoreError::NotAvailable);
                self.dirty.store(true, Ordering::Relaxed);
                Event::Done {
                    what: DoneWhat::AddressBook,
                    result,
                    ask: None,
                }
            }
            // Asked with the next snapshot (`shown_nfts`), as a due refresh is.
            Command::RefreshNfts => {
                self.nfts.ask_now();
                // FEAT-0054 part 2: a failed search of one's own collections goes again too.
                self.own_contracts_again_all();
                return (None, true);
            }
            Command::AddCollection { id } => Event::Done {
                what: DoneWhat::Collection,
                result: self.add_collection(&id).await,
                ask: None,
            },
            Command::RemoveCollection { id } => Event::Done {
                what: DoneWhat::Collection,
                result: self.remove_collection(&id),
                ask: None,
            },
            Command::TrustIssuer { issuer } => Event::Done {
                what: DoneWhat::Collection,
                result: self.trust_issuer(&issuer),
                ask: None,
            },
            Command::ReadIdentityKeys { identity } => match self.read_identity_keys(identity) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: DoneWhat::KeysRead,
                    result: Err(e),
                    ask: None,
                },
            },
            Command::FindIdentities => {
                self.discover_identities();
                return (None, true);
            }
            // Counted as the unlock is (BUG-203 r1 Must 1), and only from an open wallet: a
            // locked one would be opened by its password here without the unlock's count.
            Command::AskPhraseCheck { password } => match match self.unlocked {
                None => Err(locked()),
                Some(_) => self.open_vault_counted(&password).await,
            } {
                Ok((_, secret)) => {
                    let words: Vec<&str> = secret.phrase.split_whitespace().collect();
                    let (questions, answers) = crate::phrase_check::questions(&words, &mut os_dice);
                    // FEAT-0041 R-6: the passphrase is asked only of a wallet that has one.
                    let saved = Some(secret.passphrase.clone()).filter(|p| !p.is_empty());
                    let quiz = PhraseQuiz {
                        questions: questions.clone(),
                        passphrase: saved.is_some(),
                    };
                    self.phrase_check = Some((questions, answers, saved));
                    Event::PhraseCheck(Ok(quiz))
                }
                Err(e) => Event::PhraseCheck(Err(e)),
            },
            Command::CheckPhraseWords { picks, passphrase } => match self.phrase_check.take() {
                Some((questions, answers, saved)) => Event::PhraseChecked {
                    wrong: crate::phrase_check::first_wrong(&questions, &answers, &picks),
                    // Compared as BIP-39 salts it (NFKD); none asked of a wallet without one.
                    passphrase_wrong: saved.is_some_and(|saved| {
                        passphrase.is_none_or(|typed| {
                            *passphrase::normalized(typed.expose())
                                != *passphrase::normalized(&saved)
                        })
                    }),
                },
                // Answered already, or the wallet locked meanwhile: ask again.
                None => Event::PhraseCheck(Err(CoreError::NotAvailable(
                    tr!("The check has ended: start it again").into(),
                ))),
            },
            Command::ChooseIdentity { id } => return self.choose_identity(id).await,
            Command::CheckContract { identity, draft } => {
                match self.check_contract(identity.clone(), draft.clone()) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::ContractChecked {
                        identity,
                        draft,
                        result: Err(e),
                    },
                }
            }
            Command::PlanSeries { target, kind } => {
                match self.plan_series(target.clone(), kind.clone()) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::SeriesPlanned {
                        target,
                        kind,
                        result: Err(e),
                    },
                }
            }
            Command::PlanModel {
                identity,
                model,
                mint,
                ..
            } => match self.plan_model(identity, model, mint) {
                Ok(()) => return (None, false),
                Err(e) => Event::ModelPlanned { result: Err(e) },
            },
            Command::StopSeries { id } => {
                self.stop_series(id);
                return (None, true);
            }
            Command::CloseSeries { id } => {
                let _ = self.close_series(id);
                return (None, true);
            }
            Command::DeleteWallet { password, wallet } => {
                match wallet.filter(|id| *id != self.wallet) {
                    // Another wallet: not "delete", which the app takes for the open one gone.
                    Some(id) => Event::Done {
                        what: DoneWhat::DeleteOther,
                        result: self.delete_other(&id, password.as_ref()).await,
                        ask: None,
                    },
                    None => Event::Done {
                        what: DoneWhat::Delete,
                        result: self.delete_wallet(password.as_ref()).await,
                        ask: None,
                    },
                }
            }
            Command::Resync => Event::Done {
                what: DoneWhat::Resync,
                result: self.ask_resync().await,
                ask: None,
            },
            Command::SwitchWallet { id, password } => {
                return self.switch_wallet(id, password).await;
            }
            Command::AddWallet => {
                // The first wallet's onboarding is no adding: there is nothing to go back to.
                if self.adding_from.is_some() || self.saved.is_none() {
                    return (None, false);
                }
                // An empty folder for the onboarding; it is a wallet once its vault is written.
                // The current one stays the one opened next until then.
                self.switch_to = Some(wallets::new_id(&self.paths.root));
                self.adding_from = Some(self.wallet.clone());
                self.adding_open = self.unlocked.is_some();
                return (None, true);
            }
            Command::CancelAddWallet => match self.adding_from.clone() {
                Some(from) if self.saved.is_none() => {
                    self.switch_to = Some(from);
                    return (None, true);
                }
                _ => return (None, false),
            },
            Command::Rebroadcast { txid } => {
                let result = self.rebroadcast(txid).await;
                let changed = result.is_ok();
                return (
                    Some(Event::Done {
                        what: DoneWhat::Rebroadcast,
                        result,
                        ask: None,
                    }),
                    changed,
                );
            }
            Command::SetNote { key, text } => {
                let result = self.set_note(key, &text.0).await;
                let changed = result.is_ok();
                return (
                    Some(Event::Done {
                        what: DoneWhat::Note,
                        result,
                        ask: None,
                    }),
                    changed,
                );
            }
            Command::SetTaxTransfer { key, transfer } => {
                let result = self.set_tax_transfer(key, transfer).await;
                let changed = result.is_ok();
                return (
                    Some(Event::Done {
                        what: DoneWhat::TaxCategory,
                        result,
                        ask: None,
                    }),
                    changed,
                );
            }
            Command::TryFileAgain { file } => {
                let result = self.local_file(file, true);
                return (
                    Some(Event::Done {
                        what: DoneWhat::LocalFile,
                        result,
                        ask: None,
                    }),
                    true,
                );
            }
            Command::StartFileOver { file } => {
                let result = self.local_file(file, false);
                return (
                    Some(Event::Done {
                        what: DoneWhat::LocalFile,
                        result,
                        ask: None,
                    }),
                    true,
                );
            }
            // FEAT-0045: nothing in the snapshot changes on a save; a restore changes the notes,
            // the address book and the contacts' rows.
            Command::SaveBackup { path, settings } => {
                let result = self.save_backup(path, settings).await;
                return (Some(Event::BackupSaved(result)), false);
            }
            Command::SetAutoBackup { folder, settings } => {
                let result = self.set_auto_backup(folder, settings);
                let changed = result.is_ok();
                return (
                    Some(Event::Done {
                        what: DoneWhat::AutoBackup,
                        result,
                        ask: None,
                    }),
                    changed,
                );
            }
            Command::RestoreBackup { path } => {
                let result = self.restore_backup(path).await;
                return (Some(Event::BackupRestored(result)), true);
            }
            Command::RenameWallet { id, name, color } => {
                return self.rename_wallet(id, name, color);
            }
            Command::FindToken { id } => return self.find_token(id),
            Command::ReadWorkshop { ask } => return self.read_workshop(ask),
            Command::SearchTokens { keyword } => return self.search_tokens(keyword),
            Command::TrackToken { token_id, on } => return self.track_token(token_id, on),
            Command::FindMyTokens => return self.find_my_tokens(true),
            Command::ReadTokenHistory { token, earlier } => {
                self.read_token_history(token, earlier);
                return (None, false);
            }
            // R-22: the facts, prices among them, are read with the next snapshot.
            Command::ReadTokenPage { token_id, check } => {
                self.read_token_page(token_id, check);
                return (None, false);
            }
            Command::ReadTokenPrices => {
                ops::tokens::make_due(&self.tokens.facts_handle());
                return (None, true);
            }
        };
        (Some(event), true)
    }
}

/// FEAT-0040 (AC-T-F3): what a fee rate out of bounds is refused with.
fn fee_rate_refusal() -> CoreError {
    CoreError::NotAvailable(tr!("The fee must be from 1,000 to 1,000,000 duffs per kB").into())
}
