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

const NO_WALLET: &str = tr!("Prototype: there is no wallet behind the screens");
use evolite_common::PASSWORD_NEEDED;

/// The one password the demo's unlock refuses, so the UI tests can see "Wrong password"
/// (BUG-16); any other opens the example wallet.
pub const WRONG_PASSWORD: &str = "wrong";

/// A password the demo takes as wrong for a spend: empty, or [`WRONG_PASSWORD`].
fn refused(password: &Password) -> bool {
    password.expose().is_empty() || password.expose() == WRONG_PASSWORD
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
                }
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
            },
            // The example wallet is not on this computer to delete.
            Command::DeleteWallet { .. } => Event::Done {
                what: DoneWhat::Delete,
                result: Err(CoreError::NotAvailable(
                    tr!("This is the mockup's example: there is nothing to delete").into(),
                )),
            },
            // Nor is there a chain behind it to sync again.
            Command::Resync => Event::Done {
                what: DoneWhat::Resync,
                result: Err(CoreError::NotAvailable(
                    tr!("This is the mockup's example: there is nothing to sync").into(),
                )),
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
            },
            // The same gate as the real core's, in memory: the demo wallets take any password
            // but an empty one and [`WRONG_PASSWORD`].
            Command::Spend { op, password } => {
                match self.spend_gate(password.as_ref(), op.asks_password_always()) {
                    Err(why) => op.refused(why),
                    Ok(()) => {
                        let event = self.spend(op);
                        // A new identity or a v2 sign-in changes the wallet (FEAT-0014, FEAT-0023).
                        changed |= matches!(
                            event,
                            Event::Done {
                                what: DoneWhat::Identity | DoneWhat::App | DoneWhat::Disconnect,
                                result: Ok(())
                            }
                        );
                        // So does a payment from the Platform addresses (FEAT-0015).
                        changed |= matches!(event, Event::Sent { result: Ok(_), .. });
                        event
                    }
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
                }
            }
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
                }
            }
            Command::Lock => Event::Done {
                what: DoneWhat::Lock,
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
            },
            // The demo wallet has no keys to forget.
            Command::AutoLock { .. } | Command::Touch => return changed,
            // Nor a network to choose nodes in.
            Command::CoreNode { .. } => Event::Done {
                what: DoneWhat::CoreNode,
                result: Err(CoreError::NotAvailable(
                    tr!("This is the mockup's example: it does not connect to the network").into(),
                )),
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
            // The demo shows one address either way.
            Command::ReceiveAddress { .. } => return changed,
            Command::ChangePassword { old, .. } => Event::Done {
                what: DoneWhat::Password,
                result: if old.expose().is_empty() {
                    Err(CoreError::WrongPassword)
                } else {
                    Err(CoreError::NotAvailable(NO_WALLET.into()))
                },
            },
            Command::RevealPhrase { password, .. } => {
                Event::PhraseRevealed(if password.expose().is_empty() {
                    Err(CoreError::WrongPassword)
                } else {
                    Err(CoreError::NotAvailable(NO_WALLET.into()))
                })
            }
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
        let why = match op {
            Spend::Shield { .. } => {
                self.shield_started = Some(Instant::now());
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
                };
            }
            Spend::CreateIdentity { name, top_up } => {
                let result = self.create_identity(name.as_deref(), top_up);
                return Event::Done {
                    what: DoneWhat::Identity,
                    result,
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
                voting_days_left: None,
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
            },
        );
        Ok(())
    }

    /// A payment from the Platform addresses in the demo (FEAT-0015), as the real core makes
    /// it: the recipient chooses the operation; a foreign Core address or name is refused; the
    /// addresses give what [`address_plan`] says, the line goes into the history.
    fn pay_from_addresses(&mut self, to: &str, amount: Duffs) -> Result<SentPayment, CoreError> {
        use evolite_api::address_plan::{
            AddressMove, AddressRefusal, CREDITS_PER_DUFF, address_plan, duffs_up, move_for,
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
        let kind = move_for(to);
        let line_kind = match kind {
            AddressMove::Transfer if to.starts_with("tdash1") => {
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
        let plan =
            address_plan(kind, credits, false, &wallet.platform_inputs).map_err(
                |why| match why {
                    AddressRefusal::NotEnough { .. } => CoreError::NotEnough,
                    AddressRefusal::BelowLeast { .. } => CoreError::BadAmount,
                },
            )?;
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
            },
        );
        Ok(SentPayment {
            txid: String::new(),
            amount,
            fee,
            relayed: true,
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
        // The proof for "Hide in the pool": about six seconds, then the honest "no wallet".
        if let Some(started) = self.shield_started {
            let fraction = (started.elapsed().as_secs_f32() / 6.0).min(1.0);
            if fraction >= 1.0 {
                self.shield_started = None;
                let _ = out.send(Event::Done {
                    what: DoneWhat::Shield,
                    result: Err(CoreError::NotAvailable(NO_WALLET.into())),
                });
            } else {
                let _ = out.send(Event::Progress {
                    what: "shield",
                    fraction,
                });
            }
        }
        let Some(started) = self.sync_started else {
            return false;
        };
        let sync = staged_sync(started.elapsed().as_secs_f32());
        if sync.stage == SyncStage::Done {
            self.sync_started = None;
        }
        let changed = sync != self.snapshot.sync;
        self.snapshot.sync = sync;
        changed
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
            wallets: demo_wallets(now()),
            adding_wallet: false,
        },
        pending: None,
        sync_started: None,
        shield_started: None,
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
        let staged = mock.sync_started.is_some() || mock.shield_started.is_some();
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
                wallets: demo_wallets(0),
                adding_wallet: false,
            },
            pending: None,
            sync_started: None,
            shield_started: None,
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
                result: Err(CoreError::WrongPassword)
            }
        ));
        assert_eq!(m.snapshot.vault, VaultState::Locked);
        m.handle(unlock("anything else"), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Unlock,
                result: Ok(())
            }
        ));
        assert_eq!(m.snapshot.vault, VaultState::Unlocked);
    }

    fn done(rx: &Receiver<Event>) -> (DoneWhat, Result<(), CoreError>) {
        match rx.recv().unwrap() {
            Event::Done { what, result } => (what, result),
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
        assert_eq!(demo_wallet(0).balances.total(), 1_284_600_000);
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
                result: Ok(())
            }
        ));
        assert_eq!(shown(&m).as_deref(), Some("For the logo"));
        m.handle(note(&key, &"x".repeat(141)), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Note,
                result: Err(CoreError::NotAvailable(_))
            }
        ));
        assert_eq!(shown(&m).as_deref(), Some("For the logo"));
        m.handle(note(&key, "  "), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Note,
                result: Ok(())
            }
        ));
        assert_eq!(shown(&m), None);
        m.handle(note("demo:none", "x"), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Note,
                result: Err(_)
            }
        ));
        m.handle(note("", "x"), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: DoneWhat::Note,
                result: Err(_)
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
                result: Ok(())
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

    #[test]
    fn never_pretends_to_send() {
        let (mut m, tx, rx) = mock();
        m.handle(
            Command::Spend {
                op: Spend::Send {
                    id: 0,
                    from: Source::Core,
                    to: "@alice".into(),
                    amount: 1,
                    note: None,
                    rate: None,
                },
                password: None,
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
                        id: 7,
                        from: Source::PlatformAddress,
                        to: to.into(),
                        amount,
                        note: None,
                        rate: None,
                    },
                    password: None,
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
        assert_eq!(paid.fee, 6_500, "one input and one output");
        assert_eq!(before - held(&m), (1_000_000 + 6_500) * 1_000);
        let line = &m.snapshot.wallet.as_ref().unwrap().activity[0];
        assert!(matches!(line.kind, ActivityKind::AddressSent { .. }));
        assert_eq!((line.amount, line.fee), (-1_006_500, Some(6_500)));

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
                id: 0,
                from: Source::Core,
                to: "@alice".into(),
                amount: 1,
                note: None,
                rate: None,
            },
            Spend::TopUpIdentity { amount: 1 },
            Spend::ResumeTopUp,
            Spend::CreateIdentity {
                name: Some("alice".into()),
                top_up: 1,
            },
            Spend::ResumeIdentity,
            Spend::AddName {
                name: "alice".into(),
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
        ]
    }

    fn refusal(event: Event) -> (DoneWhat, CoreError) {
        match event {
            Event::Sent { result: Err(e), .. } => (DoneWhat::Send, e),
            Event::Done {
                what,
                result: Err(e),
            } => (what, e),
            other => panic!("not a refusal: {other:?}"),
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
            m.handle(Command::Spend { op, password: None }, &tx);
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
        assert_eq!(check_name("@evolite"), Ok(NameCheck::Contested));
        assert_eq!(check_name("evolite-2026"), Ok(NameCheck::Available));
        assert_eq!(check_name("ab"), Err(CoreError::BadName));
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
