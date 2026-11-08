//! The mock core: the example wallet from the Figma screens and a staged first sync. It
//! answers every command the way the real core will, and says plainly when something cannot
//! happen without a real wallet (sending, gifts, showing the phrase).

use crate::art::Artist;
use crate::bip39;
use evolite_api::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

const NO_WALLET: &str = tr!("Prototype: there is no wallet behind the screens");
use crate::PASSWORD_NEEDED;

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
pub(crate) fn example_wallet(t: Timestamp) -> Wallet {
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
    /// The pictures of the wallet's NFTs, by key ([`Nft::art`]).
    gallery: HashMap<String, crate::art::Stored>,
    /// `None` in tests: no worker process.
    artist: Option<Artist>,
    /// The wallet open before "Add wallet", for "Cancel".
    adding_from: Option<String>,
    /// Wrong passwords of the demo wallets, in memory (FEAT-0002).
    tries: crate::attempts::Ledger,
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
                    what: "create",
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
                    what: "contact",
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
                    what: "address-book",
                    result,
                }
            }
            Command::ForgetAddress { address } => {
                if let Some(wallet) = &mut self.snapshot.wallet {
                    wallet.address_book.retain(|e| e.address != address.trim());
                    changed = true;
                }
                Event::Done {
                    what: "address-book",
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
            Command::DisconnectApp { id } => {
                if let Some(wallet) = &mut self.snapshot.wallet {
                    wallet.apps.retain(|app| app.id != id);
                    changed = true;
                }
                Event::Done {
                    what: "disconnect",
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
            // The demo has one identity behind both choices.
            Command::ChooseIdentity { .. } => return changed,
            // Another demo wallet goes from the list by its own password (R-28); the open one
            // stays as it is.
            Command::DeleteWallet {
                password,
                wallet: Some(id),
            } if self.snapshot.wallets.iter().any(|w| w.id == id && !w.open) => Event::Done {
                what: "delete-other",
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
                what: "delete",
                result: Err(CoreError::NotAvailable(
                    tr!("This is the mockup's example: there is nothing to delete").into(),
                )),
            },
            // Nor is there a chain behind it to sync again.
            Command::Resync => Event::Done {
                what: "resync",
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
                Event::AppRequest(request)
            }
            Command::DeclineApp { .. } => Event::Done {
                what: "app",
                result: Ok(()),
            },
            // The same gate as the real core's, in memory: the demo wallets take any password
            // but an empty one and [`WRONG_PASSWORD`].
            Command::Spend { op, password } => match self.spend_gate(password.as_ref()) {
                Err(why) => op.refused(why),
                Ok(()) => {
                    let event = self.spend(op);
                    // A new identity changes the wallet (FEAT-0014).
                    changed |= matches!(
                        event,
                        Event::Done {
                            what: "identity",
                            result: Ok(())
                        }
                    );
                    event
                }
            },
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
                    what: "password-each-send",
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
                    what: "unlock",
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
                            what: "switch",
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
                let result = match (line, crate::note_text::kept(&text.0)) {
                    (None, _) => Err(CoreError::NotAvailable(
                        tr!("There is no such operation in the history").into(),
                    )),
                    (Some(_), Err(_)) => {
                        Err(CoreError::NotAvailable(crate::note_text::TOO_LONG.into()))
                    }
                    (Some(line), Ok(note)) => {
                        line.note = note;
                        changed = true;
                        Ok(())
                    }
                };
                Event::Done {
                    what: "note",
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
                    what: "wallet",
                    result,
                }
            }
            Command::Lock => Event::Done {
                what: "lock",
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
            },
            // The demo wallet has no keys to forget.
            Command::AutoLock { .. } | Command::Touch => return changed,
            // Nor a network to choose nodes in.
            Command::CoreNode { .. } => Event::Done {
                what: "core-node",
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
                what: "password",
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
    fn spend_gate(&self, password: Option<&Password>) -> Result<(), CoreError> {
        let Some(wallet) = &self.snapshot.wallet else {
            return Err(CoreError::NotAvailable(NO_WALLET.into()));
        };
        if !wallet.password_each_send {
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
            Spend::CreateIdentity { name, top_up } => {
                let result = self.create_identity(name.as_deref(), top_up);
                return Event::Done {
                    what: "identity",
                    result,
                };
            }
            _ => NO_WALLET,
        };
        op.refused(CoreError::NotAvailable(why.into()))
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
            .map(crate::dpns::label)
            .transpose()?;
        let slot = wallet
            .identities
            .iter()
            .map(|i| i.slot + 1)
            .max()
            .unwrap_or(0);
        let id = format!("{}Hk2Wm4nPq8rSt3uVw5xYz6aBc9dEf2gHj7kLm1n9QwP", slot + 2);
        let lock = top_up
            + wallet.fees.identity_create
            + label.as_ref().map_or(0, |_| wallet.fees.name_regular);
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
        let (layers, flow) = crate::layers::of(&kind, chain);
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
                detail: None,
            },
        );
        Ok(())
    }

    /// A demo wallet's password checked as the real core checks one (FEAT-0002): refused
    /// unchecked in a pause; empty and [`WRONG_PASSWORD`] are wrong and counted, in memory only.
    fn check_password(&mut self, id: &str, password: &Password) -> Result<(), CoreError> {
        use crate::attempts::Outcome;
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
                    what: "shield",
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
    art: crate::ArtOptions,
) {
    let mut mock = Mock {
        snapshot: Snapshot {
            network: crate::network::current(),
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
                network: crate::network::current(),
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
                what: "unlock",
                result: Err(CoreError::WrongPassword)
            }
        ));
        assert_eq!(m.snapshot.vault, VaultState::Locked);
        m.handle(unlock("anything else"), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: "unlock",
                result: Ok(())
            }
        ));
        assert_eq!(m.snapshot.vault, VaultState::Unlocked);
    }

    fn done(rx: &Receiver<Event>) -> (&'static str, Result<(), CoreError>) {
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
            assert_eq!(done(&rx), ("unlock", Err(CoreError::WrongPassword)));
            assert_eq!(main(&m).wrong_tries, n as u32 + 1);
        }
        let until = main(&m).wait_until.expect("a pause");
        assert!((29..=31).contains(&(until - now())));
        m.handle(unlock("right"), &tx);
        assert_eq!(
            done(&rx),
            ("unlock", Err(CoreError::TooManyTries { until }))
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
        assert_eq!(done(&rx), ("switch", Err(CoreError::WrongPassword)));
        assert_eq!(m.snapshot.wallets[1].wrong_tries, 1);
        assert_eq!(main(&m).wrong_tries, 5);
        // The pause over (as the core's clock sees it), a right password clears the count.
        m.tries.forget(&main(&m).id);
        m.tries.set(
            &main(&m).id,
            crate::attempts::Tries {
                failures: 5,
                wait_until: None,
            },
        );
        m.handle(unlock("right"), &tx);
        assert_eq!(done(&rx), ("unlock", Ok(())));
        assert_eq!((main(&m).wrong_tries, main(&m).wait_until), (0, None));
        m.handle(
            Command::DeleteWallet {
                password: Some(Password::new("pw".into())),
                wallet: Some(savings.clone()),
            },
            &tx,
        );
        assert_eq!(done(&rx), ("delete-other", Ok(())));
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
                what: "note",
                result: Ok(())
            }
        ));
        assert_eq!(shown(&m).as_deref(), Some("For the logo"));
        m.handle(note(&key, &"x".repeat(141)), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: "note",
                result: Err(CoreError::NotAvailable(_))
            }
        ));
        assert_eq!(shown(&m).as_deref(), Some("For the logo"));
        m.handle(note(&key, "  "), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: "note",
                result: Ok(())
            }
        ));
        assert_eq!(shown(&m), None);
        m.handle(note("demo:none", "x"), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: "note",
                result: Err(_)
            }
        ));
        m.handle(note("", "x"), &tx);
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: "note",
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
        let all_before = m.snapshot.wallet.as_ref().unwrap().balances.all_credits;
        create(&mut m, &tx, Some("satoshi-apps2"));
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Done {
                what: "identity",
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
                    from: Source::Core,
                    to: "@alice".into(),
                    amount: 1,
                    note: None,
                },
                password: None,
            },
            &tx,
        );
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Sent(Err(CoreError::NotAvailable(_)))
        ));
    }

    /// One of each spend (R-27 AC-T1).
    fn every_spend() -> Vec<Spend> {
        vec![
            Spend::Send {
                from: Source::Core,
                to: "@alice".into(),
                amount: 1,
                note: None,
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
            Spend::ApproveApp { id: 1 },
            Spend::GiftNft {
                id: "x".into(),
                to: "@alice".into(),
            },
        ]
    }

    fn refusal(event: Event) -> (&'static str, CoreError) {
        match event {
            Event::Sent(Err(e)) => ("send", e),
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
            if let Event::Sent(Err(e)) | Event::Done { result: Err(e), .. } = &event {
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
                ("password-each-send", CoreError::WrongPassword)
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

    /// The keys the Apps screen names are the identity's own, and a sign-in asks
    /// for a key the identity does not have yet.
    #[test]
    fn app_keys_are_the_identitys_keys() {
        let wallet = demo_wallet(0);
        let identity = wallet.identity.as_ref().unwrap();
        let has = |id: u32| identity.keys.iter().any(|k| k.id == id);
        for app in &wallet.apps {
            if let Some((id, level, _)) = &app.key {
                let key = identity.keys.iter().find(|k| k.id == *id);
                assert!(key.is_some_and(|k| k.level == *level), "{}", app.name);
            }
        }
        match app_request(1, "dash-key:abc", &wallet).unwrap().kind {
            AppRequestKind::Connect { key_id, .. } => assert!(!has(key_id), "a new key"),
            other => panic!("{other:?}"),
        }
        match app_request(2, "dash-st:abc", &wallet).unwrap().kind {
            AppRequestKind::Sign { key_id, .. } => assert!(has(key_id), "an existing key signs"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn decodes_app_links_and_refuses_others() {
        let wallet = demo_wallet(0);
        assert!(matches!(
            app_request(1, "dash-key:abc", &wallet).unwrap().kind,
            AppRequestKind::Connect { .. }
        ));
        assert!(matches!(
            app_request(2, "dash-st:abc", &wallet).unwrap().kind,
            AppRequestKind::Sign { .. }
        ));
        assert_eq!(
            app_request(3, "https://evil.example", &wallet),
            Err(CoreError::BadLink)
        );
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
