//! The mock core: the example wallet from the Figma screens and a staged first sync. It
//! answers every command the way the real core will, and says plainly when something cannot
//! happen without a real wallet (sending, gifts, showing the phrase).

#![forbid(unsafe_code)]

#[macro_use]
extern crate evolite_common;

#[cfg(test)]
mod layers_tests;

use evolite_api::*;
use evolite_art::Artist;
use evolite_common::bip39;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// Р1: the version the demo's update check says is out (newer than any build of this branch).
pub const DEMO_UPDATE: &str = "1.0.1";

const NO_WALLET: &str = tr!("Prototype: there is no wallet behind the screens");
const OFFLINE: &str = tr!("This is the mockup's example: it does not connect to the network");
use evolite_common::PASSWORD_NEEDED;

/// The one password the demo's unlock refuses, so the UI tests can see "Wrong password"
/// (BUG-16); any other opens the example wallet.
pub const WRONG_PASSWORD: &str = "wrong";

/// BUG-205: a token amount the demo answers as «not known to have gone through»
/// ([`CoreError::Unclear`]), as the real core does when a send's answer does not come.
pub const UNCLEAR_UNITS: u64 = 13;

/// FEAT-0020 AC-T16: the private key the demo shows — key 1, the textbook test key, on testnet;
/// never a wallet's. The UI tests look for it where it must not be.
pub const DEMO_KEY_WIF: &str = "cMahea7zqjxrtgAbB7LSGbcQUr1uX1ojuat9jZodMN87JcbXMTcA";
pub const DEMO_KEY_HEX: &str = "0000000000000000000000000000000000000000000000000000000000000001";

/// A password the demo takes as wrong for a spend: empty, or [`WRONG_PASSWORD`].
fn refused(password: &Password) -> bool {
    password.expose().is_empty() || password.expose() == WRONG_PASSWORD
}

/// `duffs` arrive on the demo's Platform address (FEAT-0015 stage 2): on its largest input,
/// as one address of the wallet.
fn credit_address(wallet: &mut Wallet, duffs: Duffs) {
    let credits = (duffs.max(0) as u64).saturating_mul(1_000);
    match wallet.platform_inputs.iter_mut().max() {
        Some(largest) => *largest += credits,
        None => wallet.platform_inputs.push(credits),
    }
    let all: u64 = wallet.platform_inputs.iter().sum();
    wallet.balances.platform_address = (all / 1_000) as Duffs;
}

/// The demo's "Check the phrase": word number, choices, the right one's place.
const DEMO_CHECK: [(usize, [&str; 3], usize); 4] = [
    (2, ["canvas", "cactus", "camera"], 0),
    (5, ["anchor", "alert", "album"], 1),
    (8, ["table", "talent", "tackle"], 2),
    (11, ["iron", "island", "ivory"], 1),
];

mod answers;
mod demo;
mod tokens;
mod workshop;

use answers::{PEOPLE, app_request, check_name};
use demo::{DEMO_PHRASE, demo_art, demo_wallet, demo_wallets, now, staged_sync, synced};

/// The demo's example wallet, for the core's own tests of what a snapshot holds.
#[cfg(test)]
pub fn example_wallet(t: Timestamp) -> Wallet {
    demo_wallet(t)
}

struct Mock {
    snapshot: Snapshot,
    /// The phrase between "new phrase / use phrase" and "create wallet".
    pending: Option<Phrase>,
    /// When the staged first sync started.
    sync_started: Option<Instant>,
    /// When the staged proof for "Hide in the pool" started.
    shield_started: Option<Instant>,
    /// TECH-spend-flow: the number of the spend being handled, and of the shield started, for
    /// its answer later.
    spend_ask: u64,
    shield_ask: u64,
    /// When the staged "Find my tokens" started (FEAT-0019).
    tokens_started: Option<Instant>,
    next_request: u64,
    /// The last App Connect v2 request shown, to approve (FEAT-0023).
    asked_v2: Option<(u64, AppRequestKind)>,
    /// The pictures of the wallet's NFTs, by key ([`Nft::art`]).
    gallery: HashMap<String, evolite_art::Stored>,
    /// `None` in tests: no worker process.
    artist: Option<Artist>,
    /// The wallet open before "Add wallet", for "Cancel".
    adding_from: Option<String>,
    /// Wrong passwords of the demo wallets, in memory (FEAT-0002).
    tries: evolite_common::attempts::Ledger,
}

impl Mock {
    fn handle(&mut self, command: Command, out: &Sender<Event>) -> bool {
        let mut changed = false;
        let event = match command {
            Command::NewPhrase { words } => {
                let n = if words == 24 { 24 } else { 12 };
                let phrase = Phrase::new(DEMO_PHRASE[..n].iter().map(|w| w.to_string()).collect());
                self.pending = Some(phrase.clone());
                Event::NewPhrase(phrase)
            }
            Command::UsePhrase { phrase, dashpay } => {
                let words: Vec<&str> = phrase.words().iter().map(String::as_str).collect();
                let result = match bip39::check(&words) {
                    Err(_) => Err(CoreError::BadPhrase),
                    Ok(()) => {
                        self.pending = Some(phrase.clone());
                        Ok(dashpay.then(|| DashPayScan {
                            core: 841_200_000,
                            core_addresses_used: 24,
                            identity_id: Some(PEOPLE[0].id.into()),
                            identity_credits: 120_000_000,
                            identity_keys: 3,
                            name: Some("satoshi".into()),
                            contacts: 7,
                            coinjoin: Some((8, 34_000_000)),
                            core_known: true,
                        }))
                    }
                };
                Event::PhraseAccepted(result)
            }
            Command::CreateWallet { password } => {
                let result = if password.expose().is_empty() {
                    Err(CoreError::WrongPassword)
                } else {
                    // The mock does not store the phrase; it forgets it with the password.
                    self.pending = None;
                    if self.snapshot.adding_wallet {
                        // The added wallet joins the list, open; the same example is behind it.
                        let n = self.snapshot.wallets.len();
                        for w in &mut self.snapshot.wallets {
                            w.open = false;
                        }
                        self.snapshot.wallets.push(WalletEntry {
                            id: format!("{:08x}", n + 1),
                            // Named by the app, in its language, as the real core's.
                            name: String::new(),
                            color: (n % 6) as u8,
                            origin: WalletOrigin::Created,
                            created_at: now(),
                            last_opened: Some(now()),
                            open: true,
                            wrong_tries: 0,
                            wait_until: None,
                        });
                        self.snapshot.adding_wallet = false;
                        self.adding_from = None;
                        self.snapshot.vault = VaultState::Unlocked;
                        self.snapshot.wallet = Some(demo_wallet(now()));
                    }
                    self.sync_started = Some(Instant::now());
                    self.snapshot.sync = staged_sync(0.0);
                    changed = true;
                    Ok(())
                };
                Event::Done {
                    what: DoneWhat::Create,
                    result,
                    ask: None,
                }
            }
            // The example network's names (FEAT-0039): nothing is asked of a network.
            Command::SuggestNames { prefix, seq } => Event::NameSuggestions {
                seq,
                result: Ok(answers::suggest(&prefix)),
            },
            // The example network's profiles have no pictures: the switch changes nothing here.
            Command::StrangerPictures { .. } => return changed,
            // FEAT-0040…0046: kept as said; the demo never locks.
            Command::AdvancedFeatures { on } => {
                self.snapshot.advanced_features = on;
                return true;
            }
            Command::ResolveName { name } => {
                let contacts = self
                    .snapshot
                    .wallet
                    .as_ref()
                    .map_or(&[][..], |w| &w.contacts[..]);
                let result = answers::resolve(&name, contacts);
                Event::NameResolved { name, result }
            }
            // Hiding is a choice kept on this computer: the mock can do it.
            Command::HideRequest { identity_id } => {
                if let Some(wallet) = &mut self.snapshot.wallet {
                    wallet.contacts.retain(|c| {
                        !(c.identity_id == identity_id && c.state == ContactState::Incoming)
                    });
                    changed = true;
                }
                Event::Done {
                    what: DoneWhat::Contact,
                    result: Ok(()),
                    ask: None,
                }
            }
            Command::PreviewPayment { to } => {
                let contacts = self
                    .snapshot
                    .wallet
                    .as_ref()
                    .map_or(&[][..], |w| &w.contacts[..]);
                let result = answers::preview(&to, contacts);
                Event::PaymentPreview { to, result }
            }
            // The example's coins in every state (FEAT-0013): without a wallet, none.
            Command::ListCoins { ask } => Event::Coins {
                ask,
                result: match &self.snapshot.wallet {
                    Some(_) => Ok(demo::demo_coins(now())),
                    None => Err(CoreError::NotAvailable(NO_WALLET.into())),
                },
            },
            // FEAT-0030: the example's mixed coins, as the real core plans them.
            Command::PreviewMixed { ask, amount, .. } => Event::MixedPreview {
                ask,
                result: match &self.snapshot.wallet {
                    Some(_) => demo::mixed_quote(amount),
                    None => Err(CoreError::NotAvailable(NO_WALLET.into())),
                },
            },
            // The address book is kept on this computer: the mock can keep it too (in memory).
            Command::SaveAddress { address, label } => {
                let label = label.trim().to_owned();
                let result = if label.is_empty() || label.chars().count() > 40 {
                    Err(CoreError::NotAvailable(
                        tr!("A label of 1 to 40 characters").into(),
                    ))
                } else if !address.trim().starts_with('y') {
                    Err(CoreError::BadAddress)
                } else {
                    if let Some(wallet) = &mut self.snapshot.wallet {
                        let book = &mut wallet.address_book;
                        match book.iter_mut().find(|e| e.address == address.trim()) {
                            Some(e) => e.label = label,
                            None => book.push(SavedAddress {
                                address: address.trim().into(),
                                label,
                            }),
                        }
                        book.sort_by_key(|e| e.label.to_lowercase());
                        changed = true;
                    }
                    Ok(())
                };
                Event::Done {
                    what: DoneWhat::AddressBook,
                    result,
                    ask: None,
                }
            }
            Command::ForgetAddress { address } => {
                if let Some(wallet) = &mut self.snapshot.wallet {
                    wallet.address_book.retain(|e| e.address != address.trim());
                    changed = true;
                }
                Event::Done {
                    what: DoneWhat::AddressBook,
                    result: Ok(()),
                    ask: None,
                }
            }
            Command::DrawArt { art, side, urgent } => {
                match (self.gallery.get(&art), &self.artist) {
                    (Some(stored), Some(artist)) => {
                        artist.draw(art, side, urgent, stored.clone());
                        return changed;
                    }
                    (None, _) => Event::Art {
                        art,
                        side,
                        partial: false,
                        result: Err(CoreError::Failed(tr!("no such picture").into())),
                    },
                    (Some(_), None) => Event::Art {
                        art,
                        side,
                        partial: false,
                        result: Err(CoreError::NotAvailable(NO_WALLET.into())),
                    },
                }
            }
            // A key the demo has not read yet has used 40% of its budget; one read keeps its own.
            Command::ForgetApp { id } => {
                let result = match &mut self.snapshot.wallet {
                    Some(wallet)
                        if wallet
                            .apps
                            .iter()
                            .any(|a| a.id == id && a.disconnected_at.is_some()) =>
                    {
                        wallet.apps.retain(|a| a.id != id);
                        changed = true;
                        Ok(())
                    }
                    _ => Err(CoreError::NotAvailable(
                        tr!("Disconnect the app first: its keys are still on").into(),
                    )),
                };
                Event::Done {
                    what: DoneWhat::ForgetApp,
                    result,
                    ask: None,
                }
            }
            Command::ReadAppBudgets { id } => {
                if let Some(wallet) = &mut self.snapshot.wallet {
                    for app in wallet.apps.iter_mut().filter(|a| a.id == id) {
                        for key in app.session.iter_mut().filter(|k| k.remaining.is_none()) {
                            key.remaining = Some(key.budget * 6 / 10);
                        }
                    }
                    changed = true;
                }
                Event::Done {
                    what: DoneWhat::AppBudgets,
                    result: Ok(()),
                    ask: None,
                }
            }
            // FEAT-0021 Should 1: «×» on Overview's notice, kept until the result.
            Command::CloseNameNotice { identity, label } => {
                if let Some(c) = self
                    .snapshot
                    .wallet
                    .as_mut()
                    .and_then(|w| w.identity.as_mut())
                    .filter(|i| i.id == identity)
                    .and_then(|i| i.names.iter_mut().find(|n| n.label == label))
                    .and_then(|n| {
                        let ended = !n.state.open();
                        n.contest.as_mut().map(|c| (c, ended))
                    })
                {
                    let (contest, ended) = c;
                    if ended {
                        contest.result_notice_closed = true;
                    } else {
                        contest.notice_closed = true;
                    }
                }
                return true;
            }
            Command::HideNameRequest { identity, label } => {
                let result = self.hide_name(&identity, &label);
                changed = result.is_ok();
                Event::Done {
                    what: DoneWhat::Name,
                    result,
                    ask: None,
                }
            }
            Command::CheckName { label } => Event::NameChecked {
                result: check_name(&label),
                label,
            },
            // The demo identity is always found.
            Command::FindIdentities => return changed,
            // The demo's NFTs are always there: asked again, the list is as good as new.
            Command::RefreshNfts => {
                if let Some(wallet) = &mut self.snapshot.wallet {
                    wallet.nfts_status = NftsStatus::Ready;
                }
                return true;
            }
            // The demo has one identity behind both choices: it takes the chosen id.
            Command::ChooseIdentity { id } => {
                let Some(wallet) = &mut self.snapshot.wallet else {
                    return changed;
                };
                if wallet.identities.iter().any(|i| i.id == id)
                    && let Some(identity) = wallet.identity.as_mut()
                {
                    identity.id = id;
                    return true;
                }
                return changed;
            }
            // Another demo wallet goes from the list by its own password (R-28); the open one
            // stays as it is.
            Command::DeleteWallet {
                password,
                wallet: Some(id),
            } if self.snapshot.wallets.iter().any(|w| w.id == id && !w.open) => Event::Done {
                what: DoneWhat::DeleteOther,
                result: if password.is_none_or(|p| p.expose().is_empty()) {
                    Err(CoreError::WrongPassword)
                } else {
                    self.snapshot.wallets.retain(|w| w.id != id);
                    self.tries.forget(&id);
                    changed = true;
                    Ok(())
                },
                ask: None,
            },
            // The example wallet is not on this computer to delete.
            Command::DeleteWallet { .. } => Event::Done {
                what: DoneWhat::Delete,
                result: Err(CoreError::NotAvailable(
                    tr!("This is the mockup's example: there is nothing to delete").into(),
                )),
                ask: None,
            },
            // Nor is there a chain behind it to sync again.
            Command::Resync => Event::Done {
                what: DoneWhat::Resync,
                result: Err(CoreError::NotAvailable(
                    tr!("This is the mockup's example: there is nothing to sync").into(),
                )),
                ask: None,
            },
            Command::OpenAppLink { text } => {
                self.next_request += 1;
                let request = match &self.snapshot.wallet {
                    Some(wallet) => app_request(self.next_request, &text, wallet),
                    None => Err(CoreError::NotAvailable(NO_WALLET.into())),
                };
                if let Ok(
                    asked @ AppRequest {
                        kind: AppRequestKind::Connect2 { .. },
                        ..
                    },
                ) = &request
                {
                    self.asked_v2 = Some((asked.id, asked.kind.clone()));
                }
                Event::AppRequest(request)
            }
            Command::DeclineApp { .. } => Event::Done {
                what: DoneWhat::App,
                result: Ok(()),
                ask: None,
            },
            // The same gate as the real core's, in memory: the demo wallets take any password
            // but an empty one and [`WRONG_PASSWORD`].
            // TECH-spend-flow: the answer carries the window's number back.
            Command::Spend {
                op,
                password,
                ask,
                large_confirmed,
            } => {
                self.spend_ask = ask;
                // FEAT-0004 FR-7 (AC-T8): the real core's threshold, before the password.
                let gate = if self.large_refused(&op, large_confirmed) {
                    Err(CoreError::LargeSpendNotConfirmed)
                } else {
                    self.spend_gate(password.as_ref(), op.asks_password_always())
                };
                let event = match gate {
                    Err(why) => op.refused(why),
                    Ok(()) => {
                        let event = self.spend(op);
                        // A new identity or a v2 sign-in changes the wallet (FEAT-0014, FEAT-0023).
                        changed |= matches!(
                            event,
                            Event::Done {
                                what: DoneWhat::Identity
                                    | DoneWhat::App
                                    | DoneWhat::Disconnect
                                    | DoneWhat::FundAddress
                                    | DoneWhat::Keys,
                                result: Ok(()),
                                ..
                            }
                        );
                        // So does a payment from the Platform addresses (FEAT-0015).
                        changed |= matches!(event, Event::Sent { result: Ok(_), .. });
                        event
                    }
                };
                event.answering(ask)
            }
            // FEAT-0004 FR-7 (AC-T8): as the real core — lower or on without the password,
            // higher or off with it.
            Command::SetLargeSpend { duffs, password } => {
                let result = match &mut self.snapshot.wallet {
                    None => Err(CoreError::NotAvailable(NO_WALLET.into())),
                    Some(wallet)
                        if evolite_api::confirm::weakens(wallet.large_spend, duffs)
                            && password.as_ref().is_none_or(refused) =>
                    {
                        Err(CoreError::WrongPassword)
                    }
                    Some(wallet) => {
                        changed = wallet.large_spend != duffs;
                        wallet.large_spend = duffs;
                        Ok(())
                    }
                };
                Event::Done {
                    what: DoneWhat::LargeSpend,
                    result,
                    ask: None,
                }
            }
            Command::SetPasswordEachSend { on, password } => {
                let result = match &mut self.snapshot.wallet {
                    None => Err(CoreError::NotAvailable(NO_WALLET.into())),
                    Some(_) if !on && password.as_ref().is_none_or(refused) => {
                        Err(CoreError::WrongPassword)
                    }
                    Some(wallet) => {
                        changed = wallet.password_each_send != on;
                        wallet.password_each_send = on;
                        Ok(())
                    }
                };
                Event::Done {
                    what: DoneWhat::PasswordEachSend,
                    result,
                    ask: None,
                }
            }
            // The demo wallets have no password: any opens them, but an empty one and
            // [`WRONG_PASSWORD`]; wrong ones count as in the real core (FEAT-0002).
            Command::Unlock { password } => {
                let id = self
                    .snapshot
                    .wallets
                    .iter()
                    .find(|w| w.open)
                    .map(|w| w.id.clone())
                    .unwrap_or_default();
                let result = self.check_password(&id, &password);
                if result.is_ok() {
                    self.snapshot.vault = VaultState::Unlocked;
                    if self.snapshot.wallet.is_none() {
                        self.snapshot.wallet = Some(demo_wallet(now()));
                    }
                }
                changed = true;
                Event::Done {
                    what: DoneWhat::Unlock,
                    result,
                    ask: None,
                }
            }
            // Another demo wallet opens at its unlock screen, as the real core's does (locked,
            // nothing of it shown); the same example is behind each.
            // With a password (Figma 28, 29) it opens at once, checked as by `Unlock`.
            Command::SwitchWallet { id, password } => {
                if !self.snapshot.wallets.iter().any(|w| w.id == id) {
                    return changed;
                }
                changed = true;
                match password {
                    Some(password) => {
                        let result = self.check_password(&id, &password);
                        if result.is_ok() {
                            for w in &mut self.snapshot.wallets {
                                w.open = w.id == id;
                            }
                            self.snapshot.vault = VaultState::Unlocked;
                            self.snapshot.wallet = Some(demo_wallet(now()));
                        }
                        Event::Done {
                            what: DoneWhat::Switch,
                            result,
                            ask: None,
                        }
                    }
                    None => {
                        for w in &mut self.snapshot.wallets {
                            w.open = w.id == id;
                        }
                        self.snapshot.vault = VaultState::Locked;
                        self.snapshot.wallet = None;
                        return true;
                    }
                }
            }
            // The new wallet's onboarding: none of the list is open meanwhile.
            Command::AddWallet => {
                if self.snapshot.adding_wallet {
                    return changed;
                }
                self.adding_from = self
                    .snapshot
                    .wallets
                    .iter()
                    .find(|w| w.open)
                    .map(|w| w.id.clone());
                for w in &mut self.snapshot.wallets {
                    w.open = false;
                }
                self.snapshot.adding_wallet = true;
                self.snapshot.vault = VaultState::NoWallet;
                self.snapshot.wallet = None;
                return true;
            }
            Command::CancelAddWallet => {
                if !self.snapshot.adding_wallet {
                    return changed;
                }
                self.pending = None;
                let from = self.adding_from.take();
                for w in &mut self.snapshot.wallets {
                    w.open = Some(&w.id) == from.as_ref();
                }
                self.snapshot.adding_wallet = false;
                self.snapshot.vault = VaultState::Locked;
                return true;
            }
            // In memory only (FR-11): the demo writes no file.
            Command::SetNote { key, text } => {
                let line = self.snapshot.wallet.as_mut().and_then(|w| {
                    w.activity
                        .iter_mut()
                        .find(|a| !key.is_empty() && a.key == key)
                });
                let result = match (line, evolite_common::note_text::kept(&text.0)) {
                    (None, _) => Err(CoreError::NotAvailable(
                        tr!("There is no such operation in the history").into(),
                    )),
                    (Some(_), Err(_)) => Err(CoreError::NotAvailable(
                        evolite_common::note_text::TOO_LONG.into(),
                    )),
                    (Some(line), Ok(note)) => {
                        line.note = note;
                        changed = true;
                        Ok(())
                    }
                };
                Event::Done {
                    what: DoneWhat::Note,
                    result,
                    ask: None,
                }
            }
            // FEAT-0012: in memory only, as the notes (FR-11).
            Command::SetTaxTransfer { key, transfer } => {
                let line = self.snapshot.wallet.as_mut().and_then(|w| {
                    w.activity
                        .iter_mut()
                        .find(|a| !key.is_empty() && a.key == key)
                });
                let result = match line {
                    None => Err(CoreError::NotAvailable(
                        tr!("There is no such operation in the history").into(),
                    )),
                    Some(line) => {
                        line.tax_transfer = transfer;
                        changed = true;
                        Ok(())
                    }
                };
                Event::Done {
                    what: DoneWhat::TaxCategory,
                    result,
                    ask: None,
                }
            }
            // BUG-204: the demo writes no file. A file set aside (a preset lays it over the
            // snapshot) still does not read on «Try again»; «Start over» goes through.
            Command::TryFileAgain { .. } => Event::Done {
                what: DoneWhat::LocalFile,
                result: Err(CoreError::NotAvailable(
                    tr!("The file still does not read").into(),
                )),
                ask: None,
            },
            Command::StartFileOver { .. } => Event::Done {
                what: DoneWhat::LocalFile,
                result: Ok(()),
                ask: None,
            },
            Command::RenameWallet { id, name, color } => {
                let name = name.trim().to_owned();
                let result = match self.snapshot.wallets.iter_mut().find(|w| w.id == id) {
                    _ if name.is_empty() || name.chars().count() > 24 => Err(
                        CoreError::NotAvailable(tr!("A wallet name is 1 to 24 characters").into()),
                    ),
                    Some(w) => {
                        w.name = name;
                        w.color = color % 6;
                        changed = true;
                        Ok(())
                    }
                    None => Err(CoreError::NotAvailable(
                        tr!("There is no such wallet").into(),
                    )),
                };
                Event::Done {
                    what: DoneWhat::Wallet,
                    result,
                    ask: None,
                }
            }
            // The Workshop's example reading (FEAT-0038 item 2); nothing is asked of a network.
            Command::ReadWorkshop { ask } => Event::Workshop {
                result: Ok(workshop::read(&ask)),
                ask,
            },
            // The example network's tokens (FEAT-0019 AC-T7); nothing is asked of a network.
            Command::FindToken { id } => Event::TokenFound {
                result: Ok(tokens::find(&id)),
                query: id,
            },
            Command::SearchTokens { keyword } => Event::TokenFound {
                result: Ok(tokens::search(&keyword)),
                query: keyword,
            },
            Command::TrackToken { token_id, on } => {
                let result = match &mut self.snapshot.wallet {
                    Some(w) => tokens::track(&mut w.tokens, &token_id, on),
                    None => Err(CoreError::NotAvailable(NO_WALLET.into())),
                };
                changed |= result.is_ok();
                Event::Done {
                    what: DoneWhat::Token,
                    result,
                    ask: None,
                }
            }
            Command::FindMyTokens => {
                if self.snapshot.wallet.is_some() && self.tokens_started.is_none() {
                    self.tokens_started = Some(Instant::now());
                }
                return changed;
            }
            // The demo's token history is in its snapshot already (FEAT-0019 T-1).
            Command::ReadTokenHistory { .. } => return changed,
            // R-22: the demo's prices are read again, as of now.
            Command::ReadTokenPrices => {
                let now_ms = (demo::now().max(0) as u64) * 1_000;
                if let Some(w) = self.snapshot.wallet.as_mut() {
                    for offer in w.tokens.rows.iter_mut().filter_map(|r| r.offer.as_mut()) {
                        offer.read_ms = now_ms;
                    }
                }
                Event::Snapshot(Box::new(self.snapshot.clone()))
            }
            // FEAT-0019 T-3: the demo's token page.
            Command::ReadTokenPage { token_id, check } => {
                let now = demo::now();
                if let Some(w) = self.snapshot.wallet.as_mut() {
                    let me = w.identity.as_ref().map_or(String::new(), |i| i.id.clone());
                    let mut page = tokens::demo_page(&token_id, &me, now);
                    page.checked = check.map(|c| tokens::demo_check(&page, &c));
                    w.tokens.page = Some(page);
                }
                Event::Snapshot(Box::new(self.snapshot.clone()))
            }
            Command::Lock => Event::Done {
                what: DoneWhat::Lock,
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
                ask: None,
            },
            // The demo wallet has no keys to forget.
            Command::AutoLock { .. } | Command::Touch => return changed,
            // Nor a network to choose nodes in.
            Command::CoreNode { .. } => Event::Done {
                what: DoneWhat::CoreNode,
                result: Err(CoreError::NotAvailable(
                    tr!("This is the mockup's example: it does not connect to the network").into(),
                )),
                ask: None,
            },
            // The example's price, so the screens show where it goes; no service is asked.
            Command::FiatRate { currency } => {
                self.snapshot.fiat = currency.map(|currency| FiatRate {
                    currency,
                    price: 4_796_150_000,
                    at: now(),
                });
                return true;
            }
            // Р1 (review r1): a fixed newer release, never the network; off, nothing.
            Command::CheckUpdate { on } => {
                self.snapshot.update = on.then(|| evolite_api::UpdateInfo {
                    version: DEMO_UPDATE.into(),
                    newer: true,
                    critical: false,
                    date: "2026-12-03".into(),
                    checked_at: now(),
                    notes_url: "https://github.com/Ulput/evolite/releases/tag/v1.0.1".into(),
                    verify_url: "https://github.com/Ulput/evolite/blob/main/docs/verify.md".into(),
                });
                return true;
            }
            // The demo shows one address either way.
            Command::ReceiveAddress { .. } => return changed,
            Command::ChangePassword { old, .. } => Event::Done {
                what: DoneWhat::Password,
                result: if old.expose().is_empty() {
                    Err(CoreError::WrongPassword)
                } else {
                    Err(CoreError::NotAvailable(NO_WALLET.into()))
                },
                ask: None,
            },
            Command::RevealPhrase { password, .. } => {
                Event::PhraseRevealed(if password.expose().is_empty() {
                    Err(CoreError::WrongPassword)
                } else {
                    Err(CoreError::NotAvailable(NO_WALLET.into()))
                })
            }
            // FEAT-0020 AC-T16: a published test key, never a wallet's, and the real core's
            // refusals; the password counted as the unlock's.
            Command::RevealIdentityKey {
                identity,
                key_id,
                password,
            } => {
                let revealed = self.reveal_identity_key(identity, key_id, &password);
                // SEC-11: the wrong passwords counted, as the unlock shows them.
                changed = true;
                Event::IdentityKeyRevealed(revealed)
            }
            // FEAT-0020: the example identity is as Platform has it.
            Command::ReadIdentityKeys { .. } => Event::Done {
                what: DoneWhat::KeysRead,
                result: Ok(()),
                ask: None,
            },
            // "Check the phrase" on the phrase of the Figma frames: the same four words each time.
            Command::AskPhraseCheck { password } => {
                Event::PhraseCheck(if password.expose().is_empty() {
                    Err(CoreError::WrongPassword)
                } else {
                    Ok(DEMO_CHECK
                        .iter()
                        .map(|(number, choices, _)| PhraseQuestion {
                            number: *number,
                            choices: choices.iter().map(|c| (*c).to_owned()).collect(),
                        })
                        .collect())
                })
            }
            Command::CheckPhraseWords { picks } => Event::PhraseChecked {
                wrong: DEMO_CHECK
                    .iter()
                    .enumerate()
                    .find(|(i, (_, choices, right))| {
                        picks.get(*i).is_none_or(|p| p != choices[*right])
                    })
                    .map(|(_, (number, _, _))| *number),
            },
        };
        let _ = out.send(event);
        changed
    }

    /// FEAT-0004 FR-7: a spend of more than the wallet's threshold on mainnet without the tick.
    fn large_refused(&self, op: &evolite_api::Spend, large_confirmed: bool) -> bool {
        let Some(wallet) = &self.snapshot.wallet else {
            return false;
        };
        self.snapshot.network == evolite_api::Network::Mainnet
            && !large_confirmed
            && evolite_api::confirm::asks_tick(op, &wallet.fees, wallet.large_spend)
    }

    /// Before anything of a spend: with "password for every send" on, a password (any but an
    /// empty one, as the demo's unlock); without one the spend is refused.
    /// `always`: as the real core, a spend that asks whatever the setting (FEAT-0023).
    fn spend_gate(&self, password: Option<&Password>, always: bool) -> Result<(), CoreError> {
        let Some(wallet) = &self.snapshot.wallet else {
            return Err(CoreError::NotAvailable(NO_WALLET.into()));
        };
        if !wallet.password_each_send && !always {
            return Ok(());
        }
        match password {
            None => Err(CoreError::NotAvailable(PASSWORD_NEEDED.into())),
            Some(p) if refused(p) => Err(CoreError::WrongPassword),
            Some(_) => Ok(()),
        }
    }

    /// A spend past the gate: the demo has no wallet behind it, so each says so plainly (the
    /// staged proof for "Hide in the pool" first).
    fn spend(&mut self, op: Spend) -> Event {
        let bad_amount = match &op {
            Spend::Shield { amount, .. }
            | Spend::Unshield { amount }
            | Spend::TopUpFromPool { amount } => *amount <= 0,
            _ => false,
        };
        if bad_amount {
            return op.refused(CoreError::BadAmount);
        }
        if let Spend::Send {
            id,
            from: Source::PlatformAddress,
            to,
            amount,
            rate,
            ..
        } = &op
        {
            let result = self.pay_from_addresses(to, *amount);
            // The payment keeps the rate «Summary» showed, as the real core does (АРХ-2).
            if result.is_ok()
                && let Some(line) = self
                    .snapshot
                    .wallet
                    .as_mut()
                    .and_then(|w| w.activity.first_mut())
            {
                line.rate = rate.clone();
            }
            return Event::Sent { id: *id, result };
        }
        // BUG-205: a token sent or bought of exactly [`UNCLEAR_UNITS`] is «not known to have gone
        // through», for the windows' tests and presets.
        if matches!(&op, Spend::SendToken { amount, .. } if *amount == UNCLEAR_UNITS)
            || matches!(&op, Spend::BuyToken { count, .. } if *count == UNCLEAR_UNITS)
        {
            return op.refused(CoreError::Unclear);
        }
        let why = match op {
            Spend::Shield { .. } => {
                self.shield_started = Some(Instant::now());
                self.shield_ask = self.spend_ask;
                return Event::Progress {
                    what: "shield",
                    fraction: 0.0,
                };
            }
            Spend::Unshield { .. } => {
                tr!("This is the mockup's example: there is nothing to unshield")
            }
            Spend::TopUpFromPool { .. } => {
                tr!("This is the mockup's example: there is nothing to top up with")
            }
            // As the real core: the keys disabled, the record kept until "Remove from the list".
            // FEAT-0020 AC-T8: the real core's plan on the example identity, with its refusals.
            Spend::UpdateKeys {
                identity,
                add,
                disable,
            } => {
                let result = self.update_keys(&identity, &add, &disable);
                return Event::Done {
                    what: DoneWhat::Keys,
                    result,
                    ask: None,
                };
            }
            // FEAT-0020-resend in the demo: what is owed goes, a line for the batch.
            Spend::ResendRequests { contact, .. } => {
                let result = self.resend_requests(contact.as_deref());
                return Event::Done {
                    what: DoneWhat::Resend,
                    result,
                    ask: None,
                };
            }
            Spend::DisconnectApp { id } => {
                if let Some(wallet) = &mut self.snapshot.wallet {
                    for app in wallet.apps.iter_mut().filter(|a| a.id == id) {
                        app.disconnected_at = Some(now());
                        for key in &mut app.session {
                            key.state = SessionKeyState::Disabled;
                        }
                    }
                }
                return Event::Done {
                    what: DoneWhat::Disconnect,
                    result: Ok(()),
                    ask: None,
                };
            }
            Spend::ApproveApp {
                id,
                grant: Some(grant),
            } => {
                let result = self.approve_v2(id, &grant);
                return Event::Done {
                    what: DoneWhat::App,
                    result,
                    ask: None,
                };
            }
            Spend::CreateIdentity { name, top_up } => {
                let result = self.create_identity(name.as_deref(), top_up);
                return Event::Done {
                    what: DoneWhat::Identity,
                    result,
                    ask: None,
                };
            }
            // FEAT-0015 stage 2 (AC-T10).
            Spend::FundAddress { amount } => {
                let result = self.fund_address(amount);
                return Event::Done {
                    what: DoneWhat::FundAddress,
                    result,
                    ask: None,
                };
            }
            Spend::ResumeFundAddress => {
                let result = self.resume_fund_address();
                return Event::Done {
                    what: DoneWhat::FundAddress,
                    result,
                    ask: None,
                };
            }
            // FEAT-0021: a regular name at once, a contested one into the vote.
            Spend::AddName {
                name,
                contest_fund,
                link,
            } => {
                let result = self.add_name(&name, contest_fund, link);
                return Event::Done {
                    what: DoneWhat::Name,
                    result,
                    ask: None,
                };
            }
            // FEAT-0021 АРХ-6: the link alone, as the real core writes it (AC-T12).
            Spend::SetNameLink { label, url } => {
                let result = self.set_name_link(&label, url);
                return Event::Done {
                    what: DoneWhat::Name,
                    result,
                    ask: None,
                };
            }
            _ => NO_WALLET,
        };
        op.refused(CoreError::NotAvailable(why.into()))
    }

    /// "Allow" on the demo's App Connect v2 request: the app joins the list with the keys and
    /// limits chosen, as the real core keeps it once Platform has the keys and the response.
    fn approve_v2(&mut self, id: u64, grant: &SessionGrant) -> Result<(), CoreError> {
        let stale = || {
            CoreError::NotAvailable(
                tr!("The request is out of date: open the app's link again").into(),
            )
        };
        let (asked_id, asked) = self.asked_v2.take().ok_or_else(stale)?;
        if asked_id != id {
            return Err(stale());
        }
        let app = answers::connected_v2(&asked, grant).ok_or(CoreError::NotAvailable(
            tr!("The app's first access cannot be left out: decline the request instead").into(),
        ))?;
        let wallet = self
            .snapshot
            .wallet
            .as_mut()
            .ok_or(CoreError::NotAvailable(NO_WALLET.into()))?;
        wallet.apps.retain(|a| a.id != app.id);
        wallet.apps.insert(0, app);
        Ok(())
    }

    /// «Add a name» in the demo (FEAT-0021), as the real core answers: a name that cannot be
    /// claimed as it stands is refused; a contested one is refused if the fund agreed to is not the
    /// one asked now (R-2), or if the credits do not hold the write and the fund (R-3); then a
    /// regular name is registered and a contested one goes into the vote, paid from the credits.
    fn add_name(
        &mut self,
        name: &str,
        contest_fund: Option<Duffs>,
        link: Option<String>,
    ) -> Result<(), CoreError> {
        let check = check_name(name)?;
        if !check.claimable() {
            return Err(CoreError::NameUnavailable(check));
        }
        let label = evolite_common::dpns::label(name)?;
        let wallet = self
            .snapshot
            .wallet
            .as_mut()
            .ok_or(CoreError::NotAvailable(NO_WALLET.into()))?;
        let fees = &wallet.fees;
        let (cost, state) = if check.contested() {
            if contest_fund != Some(fees.name_contest_fund) {
                return Err(CoreError::ContestFundChanged {
                    fund: fees.name_contest_fund,
                });
            }
            let now = evolite_common::now();
            let (join_until, contenders) = match check {
                NameCheck::Joinable {
                    contenders,
                    join_until,
                } => (join_until, contenders + 1),
                _ => (now + fees.name_join_secs as i64, 1),
            };
            let state = NameState::Voting {
                join_until,
                ends_about: join_until - fees.name_join_secs as i64 + fees.name_vote_secs as i64,
                contenders,
                ours: 0,
                lock: 0,
                abstain: 0,
            };
            (fees.name_contested, state)
        } else {
            (fees.name_regular, NameState::Registered)
        };
        if wallet.balances.credits < cost {
            return Err(CoreError::NotEnough);
        }
        let identity = wallet
            .identity
            .as_mut()
            .ok_or(CoreError::NotAvailable(NO_WALLET.into()))?;
        wallet.balances.credits -= cost;
        let contest = state.open().then(|| NameContest {
            contenders: vec![(identity.id.clone(), 0)],
            winner: None,
            fund: fees.name_contest_fund,
            sent_at: evolite_common::now(),
            checked_at: None,
            listed: true,
            notice_closed: false,
            result_notice_closed: false,
            link_written: link.is_some(),
            link,
        });
        identity.names.push(DpnsName {
            label,
            primary: false,
            state,
            contest,
        });
        Ok(())
    }

    /// The link of the shown identity's claim on `label` written, changed or removed (FEAT-0021
    /// АРХ-6, AC-T12), as the real core answers: a link that does not pass is refused in words;
    /// a name with no claim is not found.
    fn set_name_link(&mut self, label: &str, url: Option<String>) -> Result<(), CoreError> {
        let url = url
            .map(|l| evolite_common::dpns::identity_link(&l))
            .transpose()
            .map_err(|_| {
                CoreError::NotAvailable(
                    tr!("A link https:// of at most 128 characters is needed").into(),
                )
            })?;
        let contest = self
            .snapshot
            .wallet
            .as_mut()
            .and_then(|w| w.identity.as_mut())
            .and_then(|i| i.names.iter_mut().find(|n| n.label == label))
            .and_then(|n| n.contest.as_mut())
            .ok_or(CoreError::NameNotFound)?;
        contest.link_written = url.is_some();
        contest.link = url;
        Ok(())
    }

    /// «Hide» on a lost or blocked name (FEAT-0021 R-12): no longer listed among the names; another
    /// identity, or a name not lost or blocked, is refused and nothing changes.
    fn hide_name(&mut self, identity: &str, label: &str) -> Result<(), CoreError> {
        let shown = self
            .snapshot
            .wallet
            .as_mut()
            .and_then(|w| w.identity.as_mut())
            .filter(|i| i.id == identity)
            .ok_or(CoreError::NameNotFound)?;
        let at = shown
            .names
            .iter()
            .position(|n| {
                n.label == label
                    && matches!(n.state, NameState::Lost { .. } | NameState::Locked { .. })
            })
            .ok_or(CoreError::NameNotFound)?;
        // Kept for the card History opens, no longer listed (AC-T19).
        if let Some(contest) = &mut shown.names[at].contest {
            contest.listed = false;
        }
        Ok(())
    }

    /// "Create an identity" in the demo (FEAT-0014): the next slot gets an identity with the
    /// name, or none, and becomes the chosen one, as the real core answers once Platform has
    /// it. Refused while a creation is cut short, as the real core refuses it.
    fn create_identity(&mut self, name: Option<&str>, top_up: Duffs) -> Result<(), CoreError> {
        let wallet = self
            .snapshot
            .wallet
            .as_mut()
            .ok_or(CoreError::NotAvailable(NO_WALLET.into()))?;
        if top_up <= 0 {
            return Err(CoreError::BadAmount);
        }
        if wallet.unfinished_identity.is_some() {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped identity — its money is already locked").into(),
            ));
        }
        let label = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(evolite_common::dpns::label)
            .transpose()?;
        let slot = wallet
            .identities
            .iter()
            .map(|i| i.slot + 1)
            .max()
            .unwrap_or(0);
        let id = format!("{}Hk2Wm4nPq8rSt3uVw5xYz6aBc9dEf2gHj7kLm1n9QwP", slot + 2);
        // BUG-93: the amount becomes credits whole, the fees are on top of it.
        let lock = wallet.fees.identity_lock_for(top_up, label.is_some());
        let keys = wallet
            .identity
            .as_ref()
            .map(|i| i.keys.clone())
            .unwrap_or_default();
        let names = label
            .iter()
            .map(|label| DpnsName {
                label: label.clone(),
                primary: true,
                state: NameState::Registered,
                contest: None,
            })
            .collect();
        wallet.identity = Some(Identity {
            id: id.clone(),
            created_at: now(),
            created_block: 0,
            revision: 0,
            key_source: format!("DIP-13 · identity {slot}"),
            names,
            keys,
            profile: None,
            slot,
        });
        wallet.identities.push(IdentityChoice {
            id: id.clone(),
            name: label.clone(),
            slot,
        });
        wallet.balances.core -= lock;
        wallet.balances.credits = top_up;
        wallet.balances.all_credits += top_up;

        let line = wallet.activity.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        let kind = ActivityKind::TopUp {
            identity: label.map_or(id, |l| format!("@{l}")),
            creates: true,
            credits: Credits::Added,
        };
        let chain = Chain::CoreToPlatform;
        let (layers, flow) = evolite_common::layers::of(&kind, chain);
        wallet.activity.insert(
            0,
            Activity {
                id: line,
                kind,
                chain,
                at: now(),
                amount: -lock,
                proof: Proof::InstantLock,
                txid: None,
                fee: None,
                block: None,
                address: None,
                layers,
                flow,
                // Stable as the demo's other keys (FEAT-0003).
                key: format!("demo:{line}"),
                note: None,
                rate: None,
                detail: None,
                tax_transfer: false,
            },
        );
        Ok(())
    }

    /// A payment from the Platform addresses in the demo (FEAT-0015), as the real core makes
    /// it: the recipient chooses the operation; a foreign Core address or name is refused; the
    /// addresses give what [`address_plan`] says, the line goes into the history.
    fn pay_from_addresses(&mut self, to: &str, amount: Duffs) -> Result<SentPayment, CoreError> {
        use evolite_api::address_plan::{
            AddressMove, AddressRefusal, CREDITS_PER_DUFF, duffs_up, move_for, plan_to,
        };
        let wallet = self
            .snapshot
            .wallet
            .as_mut()
            .ok_or(CoreError::NotAvailable(NO_WALLET.into()))?;
        if amount <= 0 {
            return Err(CoreError::BadAmount);
        }
        let to = to.trim();
        let network = evolite_common::network::current();
        let kind = move_for(to, network);
        let line_kind = match kind {
            AddressMove::Transfer if evolite_api::address::is_platform(to, network) => {
                ActivityKind::AddressSent { to: to.to_owned() }
            }
            AddressMove::Transfer => return Err(CoreError::BadAddress),
            AddressMove::Withdraw if !to.starts_with('y') => return Err(CoreError::BadAddress),
            AddressMove::Withdraw if to != wallet.core_address => {
                return Err(CoreError::NotAvailable(
                    tr!("From a Platform address to Core: only to this wallet's addresses").into(),
                ));
            }
            AddressMove::Withdraw => ActivityKind::AddressWithdrawal {
                to: to.to_owned(),
                status: Some(WithdrawalState::Queued),
            },
            AddressMove::TopUp => {
                let label = to.trim_start_matches('@').to_lowercase();
                let ours = wallet.identities.iter().any(|i| {
                    i.name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&label))
                });
                if !ours {
                    return Err(CoreError::NotAvailable(
                        tr!("From a Platform address, only an identity of this wallet can be topped up")
                            .into(),
                    ));
                }
                ActivityKind::TopUp {
                    identity: format!("@{label}"),
                    creates: false,
                    credits: Credits::Added,
                }
            }
        };
        let credits = (amount as u64).saturating_mul(CREDITS_PER_DUFF);
        let plan = plan_to(
            kind,
            credits,
            false,
            &wallet.platform_inputs,
            &wallet.platform_input_addresses,
            to,
        )
        .map_err(|why| match why {
            AddressRefusal::NotEnough { .. } => CoreError::NotEnough,
            AddressRefusal::BelowLeast { .. } => CoreError::BadAmount,
        })?;
        for (i, gives) in &plan.inputs {
            wallet.platform_inputs[*i] -= gives;
        }
        if let Some(payer) = plan.fee_payer {
            wallet.platform_inputs[payer] -= plan.fee;
        }
        let left: u64 = wallet.platform_inputs.iter().sum();
        wallet.balances.platform_address = (left / CREDITS_PER_DUFF) as Duffs;
        if kind == AddressMove::TopUp {
            wallet.balances.credits += amount;
            wallet.balances.all_credits += amount;
        }
        let fee = duffs_up(plan.fee);
        let line = wallet.activity.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        let chain = Chain::Platform;
        let (layers, flow) = evolite_common::layers::of(&line_kind, chain);
        wallet.activity.insert(
            0,
            Activity {
                id: line,
                kind: line_kind,
                chain,
                at: now(),
                amount: -(amount + fee),
                proof: Proof::Proved,
                txid: None,
                fee: Some(fee),
                block: None,
                address: None,
                layers,
                flow,
                key: format!("demo:{line}"),
                note: None,
                rate: None,
                detail: None,
                tax_transfer: false,
            },
        );
        Ok(SentPayment {
            txid: String::new(),
            amount,
            fee,
            relayed: true,
        })
    }

    /// «Top up the Platform address» in the demo (FEAT-0015 stage 2, AC-T10): the same plan
    /// and refusals as the real core ([`evolite_api::address_fund::fund_plan`]); what leaves
    /// Core is the lock and its fee, the address gets the amount, the line is «Transfer from
    /// Core to the Platform address». Refused while one is stopped, as the real core refuses it.
    fn fund_address(&mut self, amount: Duffs) -> Result<(), CoreError> {
        use evolite_api::address_fund::{FundRefusal, fund_plan};
        let wallet = self
            .snapshot
            .wallet
            .as_mut()
            .ok_or(CoreError::NotAvailable(NO_WALLET.into()))?;
        if wallet
            .unfinished_address_top_up
            .as_ref()
            .is_some_and(|u| u.holds_back)
        {
            return Err(CoreError::NotAvailable(
                tr!("First finish the stopped top-up of the Platform address — its money is already locked").into(),
            ));
        }
        let spendable = wallet.balances.core - wallet.fees.core_unsignable;
        let plan = fund_plan(amount, false, spendable, &wallet.fees).map_err(|why| match why {
            FundRefusal::BelowLeast { .. } => CoreError::BadAmount,
            FundRefusal::NotEnough { .. } => CoreError::NotEnough,
        })?;
        wallet.balances.core -= plan.total;
        credit_address(wallet, plan.arrives_at_least);
        let line = wallet.activity.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        let kind = ActivityKind::AddressFunded;
        let chain = Chain::Core;
        let (layers, flow) = evolite_common::layers::of(&kind, chain);
        wallet.activity.insert(
            0,
            Activity {
                id: line,
                kind,
                chain,
                at: now(),
                amount: -plan.total,
                proof: Proof::Proved,
                txid: None,
                fee: Some(plan.core_fee),
                block: None,
                address: None,
                layers,
                flow,
                key: format!("demo:{line}"),
                note: None,
                rate: None,
                detail: None,
                tax_transfer: false,
            },
        );
        Ok(())
    }

    /// «Continue the top-up» in the demo: the stopped funding's lock reaches the address, no new
    /// money; refused when nothing is stopped.
    fn resume_fund_address(&mut self) -> Result<(), CoreError> {
        let wallet = self
            .snapshot
            .wallet
            .as_mut()
            .ok_or(CoreError::NotAvailable(NO_WALLET.into()))?;
        let Some(unfinished) = wallet.unfinished_address_top_up.take() else {
            return Err(CoreError::NotAvailable(
                tr!("Nothing to continue: the top-up of the Platform address is already credited")
                    .into(),
            ));
        };
        let arrives = unfinished.locked - unfinished.on_top;
        credit_address(wallet, arrives.max(0));
        Ok(())
    }

    /// FEAT-0020 AC-T8: the real core's plan on the shown identity, done at once — new keys this
    /// wallet's, the disabled ones dated now — or refused in the same words.
    fn update_keys(
        &mut self,
        identity: &str,
        add: &[key_plan::KeyRequest],
        disable: &[u32],
    ) -> Result<(), CoreError> {
        if self.snapshot.vault != VaultState::Unlocked {
            return Err(CoreError::NotAvailable(tr!("The wallet is locked").into()));
        }
        let Some(id) = self
            .snapshot
            .wallet
            .as_mut()
            .and_then(|w| w.identity.as_mut())
            .filter(|i| i.id == identity)
        else {
            return Err(CoreError::NotAvailable(
                tr!("Another identity is shown now: open its keys again").into(),
            ));
        };
        let plan = key_plan::plan(&id.keys, add, disable)
            .map_err(|r| CoreError::NotAvailable(evolite_common::key_refusal::words(&r)))?;
        let at = now();
        for key in id.keys.iter_mut().filter(|k| plan.disable.contains(&k.id)) {
            key.disabled_at = Some(at);
        }
        // FEAT-0020-resend: the pair replaced — its contacts get a new request each.
        let replaces = plan.add.iter().any(|k| k.dashpay)
            && id.keys.iter().any(|k| {
                plan.disable.contains(&k.id)
                    && k.purpose == KeyPurpose::Encryption
                    && k.disabled_at == Some(at)
            });
        for new in &plan.add {
            id.keys.push(IdentityKey {
                id: new.id,
                level: new.level,
                purpose: match new.purpose {
                    key_plan::NewPurpose::Authentication if new.level == KeyLevel::Master => {
                        KeyPurpose::Identity
                    }
                    key_plan::NewPurpose::Authentication => KeyPurpose::Documents,
                    key_plan::NewPurpose::Transfer => KeyPurpose::Transfers,
                    key_plan::NewPurpose::Encryption | key_plan::NewPurpose::Decryption => {
                        KeyPurpose::Encryption
                    }
                },
                kind: KeyKind::Ecdsa,
                decrypts: new.purpose == key_plan::NewPurpose::Decryption,
                disabled_at: None,
                bound_to: new.dashpay.then(|| key_plan::DASHPAY_CONTRACT.into()),
                owner: KeyOwner::This,
            });
        }
        if replaces {
            let n = self.snapshot.wallet.as_ref().map_or(0, |w| {
                w.contacts
                    .iter()
                    .filter(|c| c.state == ContactState::Established)
                    .count() as u32
            });
            if n > 0 {
                self.resent(n, n, Vec::new());
            }
        }
        Ok(())
    }

    /// FEAT-0020-resend in the demo: `sent` of `of` requests went, `due` still owed; their fee
    /// from the credits and a line in History, as the real core writes (one a batch).
    fn resent(&mut self, sent: u32, of: u32, due: Vec<String>) {
        let Some(wallet) = self.snapshot.wallet.as_mut() else {
            return;
        };
        let fee = wallet.fees.contact_request.saturating_mul(sent as Duffs);
        wallet.balances.credits = wallet.balances.credits.saturating_sub(fee);
        let kind = ActivityKind::ContactsResent {
            sent,
            of,
            left: due.len() as u32,
        };
        let line = wallet.activity.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        let chain = Chain::Platform;
        let (layers, flow) = evolite_common::layers::of(&kind, chain);
        wallet.activity.insert(
            0,
            Activity {
                id: line,
                kind,
                chain,
                at: now(),
                amount: -fee,
                proof: Proof::Proved,
                txid: None,
                fee: Some(fee),
                block: None,
                address: None,
                layers,
                flow,
                key: format!("demo:{line}"),
                note: None,
                rate: None,
                detail: None,
                tax_transfer: false,
            },
        );
        wallet.resend = Some(evolite_api::Resend {
            due,
            done: Some((sent, of)),
            ..Default::default()
        });
    }

    /// «Send requests again» in the demo: the owed requests (or `contact`'s) go.
    fn resend_requests(&mut self, contact: Option<&str>) -> Result<(), CoreError> {
        if self.snapshot.vault != VaultState::Unlocked {
            return Err(CoreError::NotAvailable(tr!("The wallet is locked").into()));
        }
        let owed = self
            .snapshot
            .wallet
            .as_ref()
            .and_then(|w| w.resend.as_ref())
            .map(|r| r.due.clone())
            .unwrap_or_default();
        let (go, left): (Vec<String>, Vec<String>) = owed
            .into_iter()
            .partition(|c| contact.is_none_or(|one| one == c));
        if go.is_empty() {
            return Err(CoreError::NotAvailable(
                tr!("No contact is waiting for a request from this identity").into(),
            ));
        }
        let n = go.len() as u32;
        self.resent(n, n, left);
        Ok(())
    }

    /// FEAT-0020 R-10 in the demo: the key `key_id` of the shown identity, refused as the real
    /// core refuses (closed wallet, no such key, disabled, an app's, another wallet's) before
    /// the password; then [`DEMO_KEY_WIF`] — the published test key 1, never a wallet's.
    fn reveal_identity_key(
        &mut self,
        identity: String,
        key_id: u32,
        password: &Password,
    ) -> Result<RevealedKey, CoreError> {
        let refuse = |words: &str| Err(CoreError::NotAvailable(words.into()));
        if self.snapshot.vault != VaultState::Unlocked {
            return refuse(tr!("The wallet is locked"));
        }
        let key = self
            .snapshot
            .wallet
            .as_ref()
            .and_then(|w| w.identity.as_ref())
            .filter(|i| i.id == identity)
            .and_then(|i| i.keys.iter().find(|k| k.id == key_id))
            .cloned();
        let Some(key) = key else {
            return refuse(tr!("The identity has no such key"));
        };
        if key.disabled_at.is_some() {
            return refuse(tr!("The key is disabled: its private key is not shown"));
        }
        match key.owner {
            KeyOwner::This => {}
            KeyOwner::App(_) => {
                return refuse(tr!(
                    "An app's key is not shown: it is not made from the phrase"
                ));
            }
            KeyOwner::Other | KeyOwner::Unknown => {
                return refuse(tr!(
                    "This key is not from this wallet's phrase: evolite cannot show it"
                ));
            }
        }
        let wallet = self
            .snapshot
            .wallets
            .iter()
            .find(|w| w.open)
            .map(|w| w.id.clone())
            .unwrap_or_default();
        self.check_password(&wallet, password)?;
        Ok(RevealedKey {
            identity,
            key_id,
            wif: Zeroizing::new(DEMO_KEY_WIF.into()),
            hex: Zeroizing::new(DEMO_KEY_HEX.into()),
        })
    }

    /// A demo wallet's password checked as the real core checks one (FEAT-0002): refused
    /// unchecked in a pause; empty and [`WRONG_PASSWORD`] are wrong and counted, in memory only.
    fn check_password(&mut self, id: &str, password: &Password) -> Result<(), CoreError> {
        use evolite_common::attempts::Outcome;
        let (wall, at) = (now().max(0) as u64, Instant::now());
        let result = match self.tries.begin(id, wall, at) {
            Err(until) => Err(CoreError::TooManyTries {
                until: Timestamp::try_from(until).unwrap_or(Timestamp::MAX),
            }),
            Ok(before) => {
                let wrong = refused(password);
                let outcome = if wrong {
                    Outcome::Wrong
                } else {
                    Outcome::Right
                };
                self.tries.end(id, before, outcome, at);
                if wrong {
                    Err(CoreError::WrongPassword)
                } else {
                    Ok(())
                }
            }
        };
        for w in &mut self.snapshot.wallets {
            w.wrong_tries = self.tries.get(&w.id).failures;
            w.wait_until = self
                .tries
                .until(&w.id, wall, at)
                .map(|t| Timestamp::try_from(t).unwrap_or(Timestamp::MAX));
        }
        result
    }

    fn tick(&mut self, out: &Sender<Event>) -> bool {
        let tokens = self.tick_tokens();
        // The proof for "Hide in the pool": about six seconds, then the honest "no wallet".
        if let Some(started) = self.shield_started {
            let fraction = (started.elapsed().as_secs_f32() / 6.0).min(1.0);
            if fraction >= 1.0 {
                self.shield_started = None;
                let _ = out.send(Event::Done {
                    what: DoneWhat::Shield,
                    result: Err(CoreError::NotAvailable(NO_WALLET.into())),
                    ask: Some(self.shield_ask),
                });
            } else {
                let _ = out.send(Event::Progress {
                    what: "shield",
                    fraction,
                });
            }
        }
        let Some(started) = self.sync_started else {
            return tokens;
        };
        let sync = staged_sync(started.elapsed().as_secs_f32());
        if sync.stage == SyncStage::Done {
            self.sync_started = None;
        }
        let changed = sync != self.snapshot.sync;
        self.snapshot.sync = sync;
        changed || tokens
    }

    /// The staged "Find my tokens": the search's progress in the snapshot, then a whole pass.
    fn tick_tokens(&mut self) -> bool {
        let Some(started) = self.tokens_started else {
            return false;
        };
        let Some(wallet) = &mut self.snapshot.wallet else {
            self.tokens_started = None;
            return false;
        };
        if tokens::step(
            &mut wallet.tokens.search,
            started.elapsed().as_secs_f32(),
            now(),
        ) {
            self.tokens_started = None;
        }
        true
    }
}

pub fn run(
    inbox: Receiver<Command>,
    out: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    art: evolite_art::ArtOptions,
) {
    let mut mock = Mock {
        snapshot: Snapshot {
            network: evolite_common::network::current(),
            sync: synced(),
            vault: VaultState::Unlocked,
            wallet: Some(demo_wallet(now())),
            demo: true,
            elsewhere: false,
            fiat: None,
            update: None,
            advanced_features: false,
            wallets: demo_wallets(now()),
            adding_wallet: false,
        },
        pending: None,
        sync_started: None,
        shield_started: None,
        spend_ask: 0,
        shield_ask: 0,
        tokens_started: None,
        next_request: 0,
        asked_v2: None,
        gallery: demo_art(),
        artist: Some(Artist::start(out.clone(), wake.clone(), art)),
        adding_from: None,
        tries: Default::default(),
    };
    let send_snapshot = |mock: &Mock| {
        let _ = out.send(Event::Snapshot(Box::new(mock.snapshot.clone())));
    };
    send_snapshot(&mock);
    wake();
    loop {
        let (answered, changed) = match inbox.recv_timeout(Duration::from_millis(250)) {
            Ok(command) => (true, mock.handle(command, &out)),
            Err(RecvTimeoutError::Timeout) => (false, false),
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // The staged sync and proof report on every tick while they run.
        let staged = mock.sync_started.is_some()
            || mock.shield_started.is_some()
            || mock.tokens_started.is_some();
        let changed = changed | mock.tick(&out);
        if changed {
            send_snapshot(&mock);
        }
        // The window is woken only when there is something new: an idle demo costs nothing.
        if answered || changed || staged {
            wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn mock() -> (Mock, Sender<Event>, Receiver<Event>) {
        let (tx, rx) = channel();
        let m = Mock {
            snapshot: Snapshot {
                network: evolite_common::network::current(),
                sync: synced(),
                vault: VaultState::Unlocked,
                wallet: Some(demo_wallet(0)),
                demo: true,
                elsewhere: false,
                fiat: None,
                update: None,
                advanced_features: false,
                wallets: demo_wallets(0),
                adding_wallet: false,
            },
            pending: None,
            sync_started: None,
            shield_started: None,
            spend_ask: 0,
            shield_ask: 0,
            tokens_started: None,
            next_request: 0,
            asked_v2: None,
            gallery: demo_art(),
            artist: None,
            adding_from: None,
            tries: Default::default(),
        };
        (m, tx, rx)
    }

    #[test]
    fn the_demo_unlock_refuses_only_its_wrong_password() {
        let (mut m, tx, rx) = mock();
        m.snapshot.vault = VaultState::Locked;
        let unlock = |password: &str| Command::Unlock {
            password: Password::new(password.into()),
        };
        m.handle(unlock(WRONG_PASSWORD), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Unlock,
                result: Err(CoreError::WrongPassword),
                ..
            }
        ));
        assert_eq!(m.snapshot.vault, VaultState::Locked);
        m.handle(unlock("anything else"), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Unlock,
                result: Ok(()),
                ..
            }
        ));
        assert_eq!(m.snapshot.vault, VaultState::Unlocked);
    }

    fn done(rx: &Receiver<Event>) -> (DoneWhat, Result<(), CoreError>) {
        match rx.recv().unwrap() {
            Event::Done { what, result, .. } => (what, result),
            other => panic!("{other:?}"),
        }
    }

    /// FEAT-0002 (FR-9, AC-A12): the demo counts as the real core — an empty password is wrong
    /// too, the 5th starts a pause of 30 s in which even a right one is refused, each wallet
    /// apart; a right one clears the count, a deletion forgets it.
    #[test]
    fn the_demo_pauses_after_five_wrong_passwords_per_wallet() {
        let (mut m, tx, rx) = mock();
        m.snapshot.vault = VaultState::Locked;
        let unlock = |password: &str| Command::Unlock {
            password: Password::new(password.into()),
        };
        let main = |m: &Mock| m.snapshot.wallets[0].clone();
        for (n, password) in ["", WRONG_PASSWORD, "", "", ""].into_iter().enumerate() {
            assert!(
                m.handle(unlock(password), &tx),
                "the count is in the snapshot"
            );
            assert_eq!(done(&rx), (DoneWhat::Unlock, Err(CoreError::WrongPassword)));
            assert_eq!(main(&m).wrong_tries, n as u32 + 1);
        }
        let until = main(&m).wait_until.expect("a pause");
        assert!((29..=31).contains(&(until - now())));
        m.handle(unlock("right"), &tx);
        assert_eq!(
            done(&rx),
            (DoneWhat::Unlock, Err(CoreError::TooManyTries { until }))
        );
        assert_eq!(m.snapshot.vault, VaultState::Locked);
        // "Savings" by its own password: no pause there, and a wrong one counts for it alone.
        let savings = m.snapshot.wallets[1].id.clone();
        m.handle(
            Command::SwitchWallet {
                id: savings.clone(),
                password: Some(Password::new(String::new())),
            },
            &tx,
        );
        assert_eq!(done(&rx), (DoneWhat::Switch, Err(CoreError::WrongPassword)));
        assert_eq!(m.snapshot.wallets[1].wrong_tries, 1);
        assert_eq!(main(&m).wrong_tries, 5);
        // The pause over (as the core's clock sees it), a right password clears the count.
        m.tries.forget(&main(&m).id);
        m.tries.set(
            &main(&m).id,
            evolite_common::attempts::Tries {
                failures: 5,
                wait_until: None,
            },
        );
        m.handle(unlock("right"), &tx);
        assert_eq!(done(&rx), (DoneWhat::Unlock, Ok(())));
        assert_eq!((main(&m).wrong_tries, main(&m).wait_until), (0, None));
        m.handle(
            Command::DeleteWallet {
                password: Some(Password::new("pw".into())),
                wallet: Some(savings.clone()),
            },
            &tx,
        );
        assert_eq!(done(&rx), (DoneWhat::DeleteOther, Ok(())));
        assert_eq!(m.tries.get(&savings).failures, 0);
    }

    #[test]
    fn demo_balances_add_up_to_the_figma_total() {
        // Figma's 12.846, and the coins mixed in DashPay (FEAT-0030, O1: «Total» counts them).
        let b = demo_wallet(0).balances;
        let mixed = b.mixed.expect("the demo's mixed coins");
        assert_eq!(
            b.total(),
            1_284_600_000 + mixed.spendable + mixed.unfinished
        );
    }

    #[test]
    fn refuses_a_phrase_with_a_bad_checksum() {
        let (mut m, tx, rx) = mock();
        let phrase = Phrase::new(vec!["abandon".to_string(); 12]);
        m.handle(
            Command::UsePhrase {
                phrase,
                dashpay: false,
            },
            &tx,
        );
        assert!(matches!(
            rx.recv().unwrap(),
            Event::PhraseAccepted(Err(CoreError::BadPhrase))
        ));
    }

    #[test]
    fn scans_dashpay_after_a_valid_phrase() {
        let (mut m, tx, rx) = mock();
        let mut words = vec!["abandon".to_string(); 11];
        words.push("about".into());
        m.handle(
            Command::UsePhrase {
                phrase: Phrase::new(words),
                dashpay: true,
            },
            &tx,
        );
        match rx.recv().unwrap() {
            Event::PhraseAccepted(Ok(Some(scan))) => assert_eq!(scan.contacts, 7),
            other => panic!("{other:?}"),
        }
    }

    /// FEAT-0003 (FR-11): the demo adds, changes and deletes a note in memory, as the core's
    /// rules say; an operation it does not have, or a note too long, is refused.
    #[test]
    fn a_note_is_set_changed_and_deleted_in_memory() {
        let (mut m, tx, rx) = mock();
        let note = |key: &str, text: &str| Command::SetNote {
            key: key.into(),
            text: NoteText(text.into()),
        };
        let shown = |m: &Mock| m.snapshot.wallet.as_ref().unwrap().activity[0].note.clone();
        let key = m.snapshot.wallet.as_ref().unwrap().activity[0].key.clone();
        assert!(!key.is_empty() && shown(&m).is_none());
        assert!(m.handle(note(&key, " For the logo \u{202E}\n"), &tx));
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Note,
                result: Ok(()),
                ..
            }
        ));
        assert_eq!(shown(&m).as_deref(), Some("For the logo"));
        m.handle(note(&key, &"x".repeat(141)), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Note,
                result: Err(CoreError::NotAvailable(_)),
                ..
            }
        ));
        assert_eq!(shown(&m).as_deref(), Some("For the logo"));
        m.handle(note(&key, "  "), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Note,
                result: Ok(()),
                ..
            }
        ));
        assert_eq!(shown(&m), None);
        m.handle(note("demo:none", "x"), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Note,
                result: Err(_),
                ..
            }
        ));
        m.handle(note("", "x"), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Note,
                result: Err(_),
                ..
            }
        ));
    }

    fn create(m: &mut Mock, tx: &Sender<Event>, name: Option<&str>) {
        m.handle(
            Command::Spend {
                op: Spend::CreateIdentity {
                    name: name.map(Into::into),
                    top_up: 30_000_000,
                },
                password: None,
                ask: 0,
                large_confirmed: false,
            },
            tx,
        );
    }

    /// FEAT-0014 (AC-T4, AC-T6, R-7): the demo's new identity takes the next slot, is the
    /// chosen one at once, counts in all the credits, and may have no name; a cut-short
    /// creation is finished first.
    #[test]
    fn the_demo_makes_one_more_identity() {
        let (mut m, tx, rx) = mock();
        let before = m.snapshot.wallet.as_ref().unwrap().balances.clone();
        let (all_before, core_before) = (before.all_credits, before.core);
        create(&mut m, &tx, Some("satoshi-apps2"));
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Identity,
                result: Ok(()),
                ..
            }
        ));
        let w = m.snapshot.wallet.as_ref().unwrap();
        let made = w.identity.as_ref().unwrap();
        assert_eq!(
            (made.slot, made.names[0].label.as_str()),
            (2, "satoshi-apps2")
        );
        assert_eq!(w.identities.last().unwrap().id, made.id);
        // BUG-93: the amount becomes credits whole; the fees are on top of it.
        let lock = w.fees.identity_lock_for(30_000_000, true);
        assert_eq!(w.balances.core, core_before - lock);
        assert_eq!(w.balances.credits, 30_000_000, "the chosen one: the new");
        assert_eq!(w.balances.all_credits, all_before + 30_000_000);

        create(&mut m, &tx, Some("  "));
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done { result: Ok(()), .. }
        ));
        let w = m.snapshot.wallet.as_ref().unwrap();
        let nameless = w.identity.as_ref().unwrap();
        assert_eq!(nameless.slot, 3);
        assert!(nameless.names.is_empty());
        assert_eq!(w.identities.last().unwrap().name, None);
        m.snapshot.wallet.as_mut().unwrap().unfinished_identity = Some(UnfinishedIdentity {
            locked: Some(1),
            name: None,
            identity: None,
        });
        create(&mut m, &tx, None);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                result: Err(CoreError::NotAvailable(_)),
                ..
            }
        ));
        assert_eq!(m.snapshot.wallet.as_ref().unwrap().identities.len(), 4);
    }

    /// FEAT-0020 AC-T8: the demo changes the example identity's keys by the real core's plan —
    /// the main key replaced at max+1, the old one dated — and refuses in the same words.
    #[test]
    fn the_demo_changes_keys_by_the_plan() {
        use evolite_api::key_plan::KeyRequest;
        let (mut m, _tx, _rx) = mock();
        m.snapshot.vault = VaultState::Unlocked;
        let shown = |m: &Mock| {
            m.snapshot
                .wallet
                .as_ref()
                .unwrap()
                .identity
                .clone()
                .unwrap()
        };
        let id = shown(&m).id;
        let next = shown(&m).keys.iter().map(|k| k.id + 1).max().unwrap();
        assert_eq!(
            m.update_keys(&id, &[KeyRequest::ReplaceMaster], &[]),
            Ok(())
        );
        let after = shown(&m);
        let old = after.keys.iter().find(|k| k.id == 0).unwrap();
        assert!(old.disabled_at.is_some(), "the old main key dated");
        let new = after.keys.iter().find(|k| k.id == next).unwrap();
        assert_eq!(
            (new.level, new.owner.clone()),
            (KeyLevel::Master, KeyOwner::This)
        );
        assert!(matches!(
            m.update_keys(&id, &[], &[next]),
            Err(CoreError::NotAvailable(words)) if words.contains("main key")
        ));
        assert!(matches!(
            m.update_keys("someone else", &[KeyRequest::Tokens], &[]),
            Err(CoreError::NotAvailable(_))
        ));
        m.snapshot.vault = VaultState::Locked;
        assert!(
            m.update_keys(&id, &[KeyRequest::ReplaceMaster], &[])
                .is_err()
        );
    }

    /// FEAT-0019 AC-T7: the example's tokens are added, found and searched again through the
    /// commands, as the screens will use them; a token is never sent.
    #[test]
    fn the_demo_answers_the_token_commands() {
        let (mut m, tx, rx) = mock();
        let rows = |m: &Mock| m.snapshot.wallet.as_ref().unwrap().tokens.rows.len();
        let before = rows(&m);
        assert!(before > 0, "the example identity has tokens");
        m.handle(
            Command::SearchTokens {
                keyword: "coffee".into(),
            },
            &tx,
        );
        let Event::TokenFound {
            query,
            result: Ok(found),
        } = rx.recv().unwrap()
        else {
            panic!("a list of tokens");
        };
        assert_eq!((query.as_str(), found.len()), ("coffee", 2));
        let stamp = found
            .iter()
            .find(|t| evolite_api::tokens::name_in(t, "en") == "Coffee Club Stamp")
            .unwrap();
        assert!(m.handle(
            Command::TrackToken {
                token_id: stamp.token_id.clone(),
                on: true,
            },
            &tx,
        ));
        assert_eq!(done(&rx), (DoneWhat::Token, Ok(())));
        assert_eq!(rows(&m), before + 1);
        m.handle(Command::FindMyTokens, &tx);
        assert!(m.tick(&tx), "the search's progress is in the snapshot");
        assert_eq!(
            m.snapshot.wallet.as_ref().unwrap().tokens.search.state,
            evolite_api::tokens::SearchState::Running
        );
        let refused = m.spend(Spend::SendToken {
            token_id: stamp.token_id.clone(),
            to: "@alice".into(),
            amount: 1,
        });
        assert!(matches!(
            refused,
            Event::Done {
                what: DoneWhat::SendToken,
                result: Err(CoreError::NotAvailable(_)),
                ..
            }
        ));
    }

    /// FEAT-0039: the example network's names by their beginning, in DPNS's form and order,
    /// the profile's name with them; nothing for fewer than 3 characters.
    #[test]
    fn the_demo_suggests_names_by_their_beginning() {
        let (mut m, tx, rx) = mock();
        let mut ask = |prefix: &str, seq| {
            m.handle(
                Command::SuggestNames {
                    prefix: prefix.into(),
                    seq,
                },
                &tx,
            );
            let Event::NameSuggestions {
                seq: answered,
                result: Ok(found),
            } = rx.recv().unwrap()
            else {
                panic!("names");
            };
            assert_eq!(answered, seq);
            found
        };
        let labels = |found: &[NameSuggestion]| -> Vec<String> {
            found.iter().map(|n| n.label.clone()).collect()
        };
        // «AIi» is «ali» to DPNS.
        let found = ask("@AIi", 7);
        assert_eq!(labels(&found), ["alice", "alice-pay", "alina"]);
        assert_eq!(found[0].display_name.as_deref(), Some("Alice Chen"));
        assert_eq!(labels(&ask("ale", 8)), ["Alex", "alexandra"]);
        assert!(ask("al", 9).is_empty());
        assert!(ask("zzq", 10).is_empty());
    }

    /// BUG-205: [`UNCLEAR_UNITS`] of a token sent or bought is answered «not known», as the
    /// real core answers a send whose answer did not come; any other amount is not.
    #[test]
    fn a_token_amount_of_thirteen_is_not_known() {
        let (mut m, _tx, _rx) = mock();
        for op in [
            Spend::SendToken {
                token_id: "x".into(),
                to: "@alice".into(),
                amount: UNCLEAR_UNITS,
            },
            Spend::BuyToken {
                token_id: "x".into(),
                count: UNCLEAR_UNITS,
                agreed: 1,
            },
        ] {
            assert!(matches!(
                m.spend(op),
                Event::Done {
                    result: Err(CoreError::Unclear),
                    ..
                }
            ));
        }
        let other = m.spend(Spend::SendToken {
            token_id: "x".into(),
            to: "@alice".into(),
            amount: 1,
        });
        assert!(!matches!(
            other,
            Event::Done {
                result: Err(CoreError::Unclear),
                ..
            }
        ));
    }

    #[test]
    fn never_pretends_to_send() {
        let (mut m, tx, rx) = mock();
        m.handle(
            Command::Spend {
                op: Spend::Send {
                    coins: None,
                    id: 0,
                    from: Source::Core,
                    to: "@alice".into(),
                    amount: 1,
                    note: None,
                    rate: None,
                },
                password: None,
                ask: 0,
                large_confirmed: false,
            },
            &tx,
        );
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Sent {
                result: Err(CoreError::NotAvailable(_)),
                ..
            }
        ));
    }

    /// BUG-141, QA pass 23: to the address Receive shows, which holds credits itself, the demo
    /// pays from the other address and leaves that one as it was — as the real core plans it
    /// (`plan_to`), and as Platform wants: an output address is never an input.
    #[test]
    fn to_its_own_funded_address_the_demo_pays_from_the_other() {
        let (mut m, tx, rx) = mock();
        let wallet = m.snapshot.wallet.as_ref().unwrap();
        let own = wallet.platform_address.clone();
        assert_eq!(
            wallet.platform_input_addresses[0], own,
            "Receive's is funded"
        );
        let before = wallet.platform_inputs.clone();
        m.handle(
            Command::Spend {
                op: Spend::Send {
                    coins: None,
                    id: 9,
                    from: Source::PlatformAddress,
                    to: own,
                    amount: 1_000_000,
                    note: None,
                    rate: None,
                },
                password: None,
                ask: 0,
                large_confirmed: false,
            },
            &tx,
        );
        match rx.recv().unwrap() {
            Event::Sent { id: 9, result } => assert!(result.is_ok(), "{result:?}"),
            other => panic!("{other:?}"),
        }
        let after = &m.snapshot.wallet.as_ref().unwrap().platform_inputs;
        assert_eq!(after[0], before[0], "the recipient gave nothing");
        assert!(after[1] < before[1], "the other paid");
    }

    /// FEAT-0015: the demo pays from its Platform addresses as the real core does: to a
    /// Platform address and to its own Core address; a foreign Core address and a foreign
    /// name are refused, nothing is taken.
    #[test]
    fn the_demo_pays_from_its_platform_addresses() {
        let (mut m, tx, rx) = mock();
        let send = |m: &mut Mock, to: &str, amount| {
            m.handle(
                Command::Spend {
                    op: Spend::Send {
                        coins: None,
                        id: 7,
                        from: Source::PlatformAddress,
                        to: to.into(),
                        amount,
                        note: None,
                        rate: None,
                    },
                    password: None,
                    ask: 0,
                    large_confirmed: false,
                },
                &tx,
            );
            match rx.recv().unwrap() {
                Event::Sent { id: 7, result } => result,
                other => panic!("{other:?}"),
            }
        };
        let held = |m: &Mock| -> u64 {
            m.snapshot
                .wallet
                .as_ref()
                .unwrap()
                .platform_inputs
                .iter()
                .sum()
        };
        let before = held(&m);
        let paid = send(&mut m, "tdash1qz8fexample", 1_000_000).expect("a transfer");
        let fee = evolite_api::address_plan::duffs_up(
            evolite_api::address_plan::AddressMove::Transfer.fee(1),
        );
        assert_eq!(paid.fee, fee, "one input and one output");
        assert_eq!(before - held(&m), (1_000_000 + fee as u64) * 1_000);
        let line = &m.snapshot.wallet.as_ref().unwrap().activity[0];
        assert!(matches!(line.kind, ActivityKind::AddressSent { .. }));
        assert_eq!((line.amount, line.fee), (-(1_000_000 + fee), Some(fee)));

        let own = m.snapshot.wallet.as_ref().unwrap().core_address.clone();
        assert!(send(&mut m, &own, 10_000_000).is_ok(), "own Core address");
        let before = held(&m);
        for (to, why) in [
            ("yForeignAddressxxxxxxxxxxxxxxxxxx", "foreign Core"),
            ("@someone-else", "foreign name"),
        ] {
            assert!(
                matches!(send(&mut m, to, 1_000_000), Err(CoreError::NotAvailable(_))),
                "{why}"
            );
        }
        assert_eq!(held(&m), before, "nothing taken");
        assert_eq!(
            send(&mut m, "tdash1qz8fexample", 1),
            Err(CoreError::BadAmount)
        );
        assert_eq!(
            send(&mut m, "tdash1qz8fexample", 1_000_000_000),
            Err(CoreError::NotEnough)
        );
    }

    /// One of each spend (R-27 AC-T1).
    fn every_spend() -> Vec<Spend> {
        vec![
            Spend::Send {
                coins: None,
                id: 0,
                from: Source::Core,
                to: "@alice".into(),
                amount: 1,
                note: None,
                rate: None,
            },
            Spend::TopUpIdentity { amount: 1 },
            Spend::ResumeTopUp,
            Spend::FundAddress { amount: 1 },
            Spend::ResumeFundAddress,
            Spend::CreateIdentity {
                name: Some("alice".into()),
                top_up: 1,
            },
            Spend::ResumeIdentity,
            Spend::AddName {
                name: "alice".into(),
                contest_fund: None,
                link: None,
            },
            Spend::Shield {
                from: ShieldSource::Credits,
                amount: 1,
            },
            Spend::Unshield { amount: 1 },
            Spend::TopUpFromPool { amount: 1 },
            Spend::AddContact {
                name: "alice".into(),
            },
            Spend::AcceptContact {
                identity_id: "x".into(),
            },
            Spend::SaveProfile {
                display: String::new(),
                about: String::new(),
                avatar_url: None,
            },
            Spend::ApproveApp { id: 1, grant: None },
            Spend::GiftNft {
                id: "x".into(),
                to: "@alice".into(),
            },
            Spend::DisconnectApp { id: "app".into() },
            Spend::SendToken {
                token_id: "x".into(),
                to: "@alice".into(),
                amount: 1,
            },
            Spend::ClaimToken {
                token_id: "x".into(),
                scheduled: false,
            },
            Spend::BuyToken {
                token_id: "x".into(),
                count: 1,
                agreed: 1,
            },
            Spend::IssuerAction {
                token_id: "x".into(),
                op: evolite_api::tokens::IssuerOp::Pause,
            },
        ]
    }

    fn refusal(event: Event) -> (DoneWhat, CoreError) {
        match event {
            Event::Sent { result: Err(e), .. } => (DoneWhat::Send, e),
            Event::Done {
                what,
                result: Err(e),
                ..
            } => (what, e),
            other => panic!("not a refusal: {other:?}"),
        }
    }

    /// BUG-137 AC-T4: a spend is the person's own operation, not a sync: the sync card's mark
    /// does not move for it.
    #[test]
    fn a_spend_is_no_platform_sync() {
        let (mut m, _tx, _rx) = mock();
        m.snapshot.wallet = Some(demo_wallet(now()));
        for op in every_spend() {
            let _ = m.spend(op);
            assert!(m.snapshot.sync.platform.is_empty());
        }
    }

    #[test]
    fn with_a_password_for_every_send_no_spend_goes_without_it() {
        let (mut m, tx, rx) = mock();
        m.snapshot.wallet = Some(demo_wallet(now()));
        m.snapshot.wallet.as_mut().unwrap().password_each_send = true;
        let before = m.snapshot.clone();
        for op in every_spend() {
            let what = op.what();
            m.handle(
                Command::Spend {
                    op: op.clone(),
                    password: None,
                    ask: 0,
                    large_confirmed: false,
                },
                &tx,
            );
            assert_eq!(
                refusal(rx.recv().unwrap()),
                (what, CoreError::NotAvailable(PASSWORD_NEEDED.into())),
                "{op:?}"
            );
            m.handle(
                Command::Spend {
                    op: op.clone(),
                    password: Some(Password::new(String::new())),
                    ask: 0,
                    large_confirmed: false,
                },
                &tx,
            );
            assert_eq!(
                refusal(rx.recv().unwrap()),
                (what, CoreError::WrongPassword),
                "{op:?}"
            );
        }
        // Nothing started: no proof under way, the wallet as it was.
        assert!(m.shield_started.is_none());
        assert_eq!(m.snapshot, before);
        // With the password, the spend goes on to the demo's own answer.
        m.handle(
            Command::Spend {
                op: Spend::Shield {
                    from: ShieldSource::Credits,
                    amount: 1,
                },
                password: Some(Password::new("any".into())),
                ask: 0,
                large_confirmed: false,
            },
            &tx,
        );
        assert!(matches!(rx.recv().unwrap(), Event::Progress { .. }));
    }

    #[test]
    fn without_the_setting_a_spend_needs_no_password() {
        let (mut m, tx, rx) = mock();
        m.snapshot.wallet = Some(demo_wallet(now()));
        for op in every_spend() {
            m.handle(
                Command::Spend {
                    op,
                    password: None,
                    ask: 0,
                    large_confirmed: false,
                },
                &tx,
            );
            let event = rx.recv().unwrap();
            if let Event::Sent { result: Err(e), .. } | Event::Done { result: Err(e), .. } = &event
            {
                assert!(
                    !matches!(e, CoreError::WrongPassword)
                        && *e != CoreError::NotAvailable(PASSWORD_NEEDED.into()),
                    "{event:?}"
                );
            }
        }
    }

    #[test]
    fn the_setting_goes_off_only_with_the_password() {
        let (mut m, tx, rx) = mock();
        m.snapshot.wallet = Some(demo_wallet(now()));
        let on = |m: &Mock| m.snapshot.wallet.as_ref().unwrap().password_each_send;
        m.handle(
            Command::SetPasswordEachSend {
                on: true,
                password: None,
            },
            &tx,
        );
        assert!(matches!(rx.recv().unwrap(), Event::Done { result: Ok(()), .. }) && on(&m));
        for password in [None, Some(Password::new(String::new()))] {
            m.handle(
                Command::SetPasswordEachSend {
                    on: false,
                    password,
                },
                &tx,
            );
            assert_eq!(
                refusal(rx.recv().unwrap()),
                (DoneWhat::PasswordEachSend, CoreError::WrongPassword)
            );
            assert!(on(&m));
        }
        m.handle(
            Command::SetPasswordEachSend {
                on: false,
                password: Some(Password::new("any".into())),
            },
            &tx,
        );
        assert!(matches!(rx.recv().unwrap(), Event::Done { result: Ok(()), .. }) && !on(&m));
    }

    #[test]
    fn previews_where_a_payment_goes() {
        let contacts = demo_wallet(0).contacts;
        assert!(answers::preview("@alice", &contacts).is_ok_and(|a| a.starts_with('y')));
        assert_eq!(
            answers::preview(" yXk3 ", &contacts),
            Ok("yXk3".to_string()),
            "an address is itself"
        );
        // Not a contact (carol), or a request still waiting (kate): no address from a key.
        assert!(matches!(
            answers::preview("@carol", &contacts),
            Err(CoreError::NotAvailable(_))
        ));
        assert!(matches!(
            answers::preview("@kate", &contacts),
            Err(CoreError::NotAvailable(_))
        ));
    }

    /// BUG-231 AC-A1, AC-A3: a contact whose channel is broken is refused by name, in words
    /// and not «in a minute»; once the mark is gone its address is there again.
    #[test]
    fn a_broken_channel_is_refused_by_name() {
        let mut contacts = demo_wallet(0).contacts;
        let alice = contacts
            .iter_mut()
            .find(|c| c.name.as_deref() == Some("alice"))
            .unwrap();
        alice.channel_broken = true;
        let Err(CoreError::NotAvailable(why)) = answers::preview("@alice", &contacts) else {
            panic!("@alice is paid");
        };
        assert!(
            why.starts_with("You cannot pay @alice by DashPay yet"),
            "{why}"
        );
        assert!(!why.contains("minute"), "{why}");
        // Another contact is paid as before.
        assert!(answers::preview("@dash-core-dev", &contacts).is_ok());
        for c in &mut contacts {
            c.channel_broken = false;
        }
        assert!(answers::preview("@alice", &contacts).is_ok());
    }

    #[test]
    fn every_demo_nft_has_its_picture_and_no_other_is_drawn() {
        let (mut m, tx, rx) = mock();
        for nft in &demo_wallet(0).nfts {
            assert!(m.gallery.contains_key(&nft.art), "{}", nft.id);
        }
        m.handle(
            Command::DrawArt {
                art: "00".into(),
                side: 64,
                urgent: true,
            },
            &tx,
        );
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Art {
                result: Err(CoreError::Failed(_)),
                ..
            }
        ));
    }

    #[test]
    fn checks_names_by_dpns_rules() {
        assert_eq!(check_name("satoshi"), Ok(NameCheck::Taken));
        assert_eq!(check_name("@evolite"), Ok(NameCheck::NeedsVote));
        assert_eq!(check_name("evolite-2026"), Ok(NameCheck::Available));
        assert_eq!(check_name("ab"), Err(CoreError::BadName));
    }

    /// FEAT-0021 (AC-T8): the demo answers «Add a name» as the real core does — a name that
    /// cannot be claimed, a fund other than the one asked now, too few credits: refused with
    /// nothing paid; a regular name registered; a contested one in the vote with the protocol's
    /// times; «Hide» only on a lost or blocked name of the shown identity.
    #[test]
    fn the_demo_claims_names_as_the_real_core_does() {
        let (mut m, _tx, _rx) = mock();
        let credits = |m: &Mock| m.snapshot.wallet.as_ref().unwrap().balances.credits;
        let names = |m: &Mock| {
            m.snapshot
                .wallet
                .as_ref()
                .unwrap()
                .identity
                .clone()
                .unwrap()
                .names
        };
        let fund = m.snapshot.wallet.as_ref().unwrap().fees.name_contest_fund;
        let before = credits(&m);
        for name in [answers::LOCKED, answers::JOIN_CLOSED, "satoshi"] {
            assert!(
                matches!(
                    m.add_name(name, Some(fund), None),
                    Err(CoreError::NameUnavailable(
                        NameCheck::Locked | NameCheck::JoinClosed { .. } | NameCheck::Taken
                    ))
                ),
                "{name}"
            );
        }
        assert_eq!(
            m.add_name("sam", Some(fund + 1), None),
            Err(CoreError::ContestFundChanged { fund }),
            "R-2"
        );
        assert_eq!(credits(&m), before, "nothing paid");

        m.add_name("evolite-2026", None, None).unwrap();
        m.add_name("sam", Some(fund), None).unwrap();
        let fees = m.snapshot.wallet.as_ref().unwrap().fees.clone();
        assert_eq!(
            credits(&m),
            before - fees.name_regular - fees.name_contested
        );
        let all = names(&m);
        let state = |label: &str| all.iter().find(|n| n.label == label).unwrap().state;
        assert_eq!(state("evolite-2026"), NameState::Registered);
        let NameState::Voting {
            join_until,
            ends_about,
            contenders: 1,
            ..
        } = state("sam")
        else {
            panic!("in the vote: {:?}", state("sam"));
        };
        assert_eq!(
            ends_about - join_until,
            (fees.name_vote_secs - fees.name_join_secs) as i64
        );

        m.snapshot.wallet.as_mut().unwrap().balances.credits = 0;
        assert_eq!(
            m.add_name("bob2", Some(fund), None),
            Err(CoreError::NotEnough),
            "R-3"
        );

        let id = m
            .snapshot
            .wallet
            .as_ref()
            .unwrap()
            .identity
            .as_ref()
            .unwrap()
            .id
            .clone();
        assert_eq!(
            m.hide_name(&id, "sam"),
            Err(CoreError::NameNotFound),
            "still in the vote"
        );
        assert_eq!(m.hide_name("other", "sam"), Err(CoreError::NameNotFound));
    }

    /// FEAT-0021 АРХ-6 (AC-T12): a claim's link is written alone — the request stays, nothing is
    /// charged for a fund; a link that does not pass and a name not claimed are refused.
    #[test]
    fn the_demo_writes_a_claims_link_alone() {
        let (mut m, _tx, _rx) = mock();
        let fund = m.snapshot.wallet.as_ref().unwrap().fees.name_contest_fund;
        m.add_name("sam", Some(fund), None).unwrap();
        let credits = |m: &Mock| m.snapshot.wallet.as_ref().unwrap().balances.credits;
        let link = |m: &Mock| {
            let id = m
                .snapshot
                .wallet
                .as_ref()
                .unwrap()
                .identity
                .clone()
                .unwrap();
            let c = id
                .names
                .into_iter()
                .find(|n| n.label == "sam")
                .unwrap()
                .contest
                .unwrap();
            (c.link, c.link_written)
        };
        let before = credits(&m);
        assert_eq!(link(&m), (None, false));
        m.set_name_link("sam", Some("https://example.com/me".into()))
            .unwrap();
        assert_eq!(link(&m), (Some("https://example.com/me".into()), true));
        assert_eq!(credits(&m), before, "no fund again");
        assert!(
            m.set_name_link("sam", Some("http://example.com".into()))
                .is_err()
        );
        assert_eq!(
            m.set_name_link("nobody", Some("https://example.com".into())),
            Err(CoreError::NameNotFound)
        );
        m.set_name_link("sam", None).unwrap();
        assert_eq!(link(&m), (None, false), "removed");
    }

    /// FEAT-0023: v2 links are read, App Connect v1 ones refused in words of their own.
    #[test]
    fn decodes_app_links_and_refuses_others() {
        let wallet = demo_wallet(0);
        assert!(matches!(
            app_request(1, "dash-key:abc?n=t&v=2", &wallet)
                .unwrap()
                .kind,
            AppRequestKind::Connect2 { .. }
        ));
        for v1 in ["dash-key:abc", "dash-st:abc"] {
            match app_request(2, v1, &wallet) {
                Err(CoreError::AppLink(AppLinkRefusal::Version { v })) => assert_eq!(v, "1"),
                other => panic!("{v1}: {other:?}"),
            }
        }
        assert_eq!(
            app_request(3, "https://evil.example", &wallet),
            Err(CoreError::BadLink)
        );
    }

    /// FEAT-0023: the demo walks an App Connect v2 sign-in as the real core does: the request
    /// with the wallet's limits, «Allow» with the person's limits (never without the primary
    /// key), the app in the list with its session keys, and what is left of their budgets.
    #[test]
    fn a_v2_sign_in_goes_from_the_request_to_the_apps_list() {
        let (mut m, tx, rx) = mock();
        let events = || rx.try_iter().collect::<Vec<_>>();
        let opened = |events: Vec<Event>| {
            events.into_iter().find_map(|e| match e {
                Event::AppRequest(Ok(r)) => Some(r),
                _ => None,
            })
        };
        let done = |events: Vec<Event>| {
            events.into_iter().find_map(|e| match e {
                Event::Done {
                    what: DoneWhat::App,
                    result,
                    ..
                } => Some(result),
                _ => None,
            })
        };
        let approve = |id, keys| Command::Spend {
            op: Spend::ApproveApp {
                id,
                grant: Some(SessionGrant { keys }),
            },
            password: Some(Password::new("any".into())),
            ask: 0,
            large_confirmed: false,
        };
        let link = |text: &str| Command::OpenAppLink { text: text.into() };
        m.handle(link("dashpay://connect?v=2&n=t&r=AQ"), &tx);
        let request = opened(events()).expect("a request");
        let AppRequestKind::Connect2 {
            sessions, limits, ..
        } = &request.kind;
        assert_eq!(limits, &SessionLimits::wallet());
        assert!(sessions[0].primary && !sessions[1].primary);
        let end = evolite_common::now() + 30 * 24 * 60 * 60;
        m.handle(approve(request.id, vec![None, Some((1_000_000, end))]), &tx);
        assert!(
            matches!(done(events()), Some(Err(CoreError::NotAvailable(_)))),
            "not without the primary key"
        );
        // Refused, the request is gone: it is opened again, as the real core asks.
        m.handle(link("dash-key:abc?n=t&v=2"), &tx);
        let again = opened(events()).expect("a v2 dash-key: request");
        assert!(matches!(again.kind, AppRequestKind::Connect2 { .. }));
        m.handle(approve(again.id, vec![Some((20_000_000, end)), None]), &tx);
        assert_eq!(done(events()), Some(Ok(())));
        let app = m.snapshot.wallet.as_ref().unwrap().apps[0].clone();
        assert_eq!(app.session.len(), 1, "the profile key was left out");
        assert_eq!(
            (app.session[0].budget, app.session[0].expires_at),
            (20_000_000, end)
        );
        m.handle(Command::ReadAppBudgets { id: app.id.clone() }, &tx);
        let read = &m.snapshot.wallet.as_ref().unwrap().apps[0].session[0];
        assert_eq!(read.remaining, Some(12_000_000));
    }

    #[test]
    fn staged_sync_ends_with_a_chainlock() {
        assert_eq!(staged_sync(0.0).stage, SyncStage::Filters);
        assert_eq!(staged_sync(31.0).stage, SyncStage::Transactions);
        let done = staged_sync(60.0);
        assert_eq!(done.stage, SyncStage::Done);
        assert!(done.chainlocked);
    }
}
