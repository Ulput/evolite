//! The commands: which operation each one is, and the answer it gets.

use super::*;

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
            } => match self.send(id, from, &to, amount, (note, rate, coins)).await {
                // The answer comes from the broadcast task.
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
                    },
                }
            }
            Spend::ResumeIdentity => match self.resume_identity().await {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Identity,
                    result: Err(e),
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
                },
            },
            Spend::GiftNft { .. } => Event::Done {
                what: DoneWhat::Gift,
                result: Err(CoreError::NotAvailable(NO_GIFTS.into())),
            },
            Spend::AddContact { name } => match self.add_contact(&name) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: DoneWhat::Contact,
                    result: Err(e),
                },
            },
            Spend::AcceptContact { identity_id } => {
                match self.contact_job(&identity_id, ContactJob::Accept) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::Done {
                        what: DoneWhat::Contact,
                        result: Err(e),
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
                },
            },
            Spend::TopUpIdentity { amount } => match self.top_up_identity(amount).await {
                // The answer comes from the background task.
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::TopUp,
                    result: Err(e),
                },
            },
            Spend::ResumeTopUp => match self.resume_top_up() {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::TopUp,
                    result: Err(e),
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
                },
            },
            Spend::AddName { name } => match self.add_name(&name).await {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Name,
                    result: Err(e),
                },
            },
            Spend::DisconnectApp { id } => match self.disconnect_app(&id) {
                Ok(()) => return (None, true),
                Err(e) => Event::Done {
                    what: DoneWhat::Disconnect,
                    result: Err(e),
                },
            },
            Spend::ApproveApp { id, grant } => match self.answer_app(id, true, grant) {
                // The answer comes from the background task.
                Ok(true) => return (None, true),
                Ok(false) => Event::Done {
                    what: DoneWhat::App,
                    result: Ok(()),
                },
                Err(e) => Event::Done {
                    what: DoneWhat::App,
                    result: Err(e),
                },
            },
        };
        (Some(event), true)
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
            },
            Command::CreateWallet { password } => Event::Done {
                what: DoneWhat::Create,
                result: self.create(&password).await,
            },
            Command::Unlock { password } => Event::Done {
                what: DoneWhat::Unlock,
                result: self.unlock_counted(&password).await,
            },
            Command::Lock => Event::Done {
                what: DoneWhat::Lock,
                result: self.lock().await,
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
            Command::ReceiveAddress { fresh } => {
                self.fresh_receive = fresh;
                // The next snapshot takes the address again.
                self.receive_address = None;
                return (None, true);
            }
            Command::ChangePassword { old, new } => Event::Done {
                what: DoneWhat::Password,
                result: self.change_password(&old, &new).await,
            },
            Command::RevealPhrase { password, wallet } => {
                let opened = match wallet.filter(|id| *id != self.wallet) {
                    Some(id) => self.open_wallet_vault(&id, &password).await,
                    None => self.open_vault(&password).await,
                };
                let result = opened.map(|(_, phrase)| {
                    Phrase::new(phrase.split_whitespace().map(str::to_owned).collect())
                });
                Event::PhraseRevealed(result)
            }
            Command::ResolveName { name } => return self.resolve_name(name).await,
            Command::CheckName { label: asked } => return self.check_name(asked).await,
            Command::OpenAppLink { text } => match self.open_app_link(&text) {
                // The decoded request comes from the background task.
                Ok(()) => return (None, false),
                Err(e) => Event::AppRequest(Err(e)),
            },
            // NFT pictures as their documents store them (packed), or profile pictures: a
            // plain image file, never packed.
            Command::DrawArt { art, side, urgent } => return self.draw_art(art, side, urgent),
            Command::HideRequest { identity_id } => {
                match self.contact_job(&identity_id, ContactJob::Hide) {
                    Ok(()) => return (None, false),
                    Err(e) => Event::Done {
                        what: DoneWhat::Contact,
                        result: Err(e),
                    },
                }
            }
            Command::ListCoins { ask } => Event::Coins {
                ask,
                result: self.list_coins().await,
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
                },
            },
            Command::ReadAppBudgets { id } => match self.read_app_budgets(&id) {
                Ok(()) => return (None, false),
                Err(e) => Event::Done {
                    what: DoneWhat::AppBudgets,
                    result: Err(e),
                },
            },
            // Every spend passes the one gate first (R-27): refused, it is answered as the
            // operation's own failure, and nothing of it was checked, locked or signed.
            Command::Spend { op, password } => {
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
                return (event, changed);
            }
            Command::SetPasswordEachSend { on, password } => Event::Done {
                what: DoneWhat::PasswordEachSend,
                result: self.set_password_each_send(on, password.as_ref()).await,
            },
            // Turning a request down spends nothing: no password.
            Command::DeclineApp { id } => match self.answer_app(id, false, None) {
                Ok(_) => Event::Done {
                    what: DoneWhat::App,
                    result: Ok(()),
                },
                Err(e) => Event::Done {
                    what: DoneWhat::App,
                    result: Err(e),
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
                }
            }
            // Asked with the next snapshot (`shown_nfts`), as a due refresh is.
            Command::RefreshNfts => {
                self.nfts.ask_now();
                return (None, true);
            }
            Command::FindIdentities => {
                self.discover_identities();
                return (None, true);
            }
            Command::AskPhraseCheck { password } => match self.open_vault(&password).await {
                Ok((_, phrase)) => {
                    let words: Vec<&str> = phrase.split_whitespace().collect();
                    let (questions, answers) = crate::phrase_check::questions(&words, &mut os_dice);
                    self.phrase_check = Some((questions.clone(), answers));
                    Event::PhraseCheck(Ok(questions))
                }
                Err(e) => Event::PhraseCheck(Err(e)),
            },
            Command::CheckPhraseWords { picks } => match self.phrase_check.take() {
                Some((questions, answers)) => Event::PhraseChecked {
                    wrong: crate::phrase_check::first_wrong(&questions, &answers, &picks),
                },
                // Answered already, or the wallet locked meanwhile: ask again.
                None => Event::PhraseCheck(Err(CoreError::NotAvailable(
                    tr!("The check has ended: start it again").into(),
                ))),
            },
            Command::ChooseIdentity { id } => return self.choose_identity(id).await,
            Command::DeleteWallet { password, wallet } => {
                match wallet.filter(|id| *id != self.wallet) {
                    // Another wallet: not "delete", which the app takes for the open one gone.
                    Some(id) => Event::Done {
                        what: DoneWhat::DeleteOther,
                        result: self.delete_other(&id, password.as_ref()).await,
                    },
                    None => Event::Done {
                        what: DoneWhat::Delete,
                        result: self.delete_wallet(password.as_ref()).await,
                    },
                }
            }
            Command::Resync => Event::Done {
                what: DoneWhat::Resync,
                result: self.ask_resync().await,
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
                return (None, true);
            }
            Command::CancelAddWallet => match self.adding_from.clone() {
                Some(from) if self.saved.is_none() => {
                    self.switch_to = Some(from);
                    return (None, true);
                }
                _ => return (None, false),
            },
            Command::SetNote { key, text } => {
                let result = self.set_note(key, &text.0).await;
                let changed = result.is_ok();
                return (
                    Some(Event::Done {
                        what: DoneWhat::Note,
                        result,
                    }),
                    changed,
                );
            }
            Command::RenameWallet { id, name, color } => {
                return self.rename_wallet(id, name, color);
            }
        };
        (Some(event), true)
    }
}
