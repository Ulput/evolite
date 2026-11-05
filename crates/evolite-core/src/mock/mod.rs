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

/// The one password the demo's unlock refuses, so the UI tests can see "Wrong password"
/// (BUG-16); any other opens the example wallet.
pub const WRONG_PASSWORD: &str = "wrong";

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
            Command::AddContact { .. } | Command::AcceptContact { .. } => Event::Done {
                what: "contact",
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
            },
            Command::SaveProfile { .. } => Event::Done {
                what: "profile",
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
            },
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
            Command::Send { .. } => Event::Sent(Err(CoreError::NotAvailable(NO_WALLET.into()))),
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
            Command::GiftNft { .. } => Event::Done {
                what: "gift",
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
            },
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
            Command::CreateIdentity { .. } => Event::Done {
                what: "identity",
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
            },
            Command::ResumeIdentity => Event::Done {
                what: "identity",
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
            },
            Command::TopUpIdentity { .. } | Command::ResumeTopUp => Event::Done {
                what: "top-up",
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
            },
            Command::AddName { .. } => Event::Done {
                what: "name",
                result: Err(CoreError::NotAvailable(NO_WALLET.into())),
            },
            // The demo identity is always found.
            Command::FindIdentities => return changed,
            // The demo has one identity behind both choices.
            Command::ChooseIdentity { .. } => return changed,
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
            Command::Shield { amount, .. } => {
                if amount <= 0 {
                    Event::Done {
                        what: "shield",
                        result: Err(CoreError::BadAmount),
                    }
                } else {
                    self.shield_started = Some(Instant::now());
                    Event::Progress {
                        what: "shield",
                        fraction: 0.0,
                    }
                }
            }
            Command::Unshield { amount } => Event::Done {
                what: "unshield",
                result: if amount <= 0 {
                    Err(CoreError::BadAmount)
                } else {
                    Err(CoreError::NotAvailable(
                        tr!("This is the mockup's example: there is nothing to unshield").into(),
                    ))
                },
            },
            Command::TopUpFromPool { amount } => Event::Done {
                what: "pool-top-up",
                result: if amount <= 0 {
                    Err(CoreError::BadAmount)
                } else {
                    Err(CoreError::NotAvailable(
                        tr!("This is the mockup's example: there is nothing to top up with").into(),
                    ))
                },
            },
            Command::OpenAppLink { text } => {
                self.next_request += 1;
                let request = match &self.snapshot.wallet {
                    Some(wallet) => app_request(self.next_request, &text, wallet),
                    None => Err(CoreError::NotAvailable(NO_WALLET.into())),
                };
                Event::AppRequest(request)
            }
            Command::AnswerApp { approve, .. } => Event::Done {
                what: "app",
                result: if approve {
                    Err(CoreError::NotAvailable(NO_WALLET.into()))
                } else {
                    Ok(())
                },
            },
            // The demo wallets have no password: any opens them, but for [`WRONG_PASSWORD`].
            Command::Unlock { password } if password.expose() == WRONG_PASSWORD => Event::Done {
                what: "unlock",
                result: Err(CoreError::WrongPassword),
            },
            Command::Unlock { .. } => {
                changed = self.snapshot.vault != VaultState::Unlocked;
                self.snapshot.vault = VaultState::Unlocked;
                if self.snapshot.wallet.is_none() {
                    self.snapshot.wallet = Some(demo_wallet(now()));
                }
                Event::Done {
                    what: "unlock",
                    result: Ok(()),
                }
            }
            // Another demo wallet opens at its unlock screen, as the real core's does (locked,
            // nothing of it shown); the same example is behind each.
            // With a password (Figma 28, 29) it opens at once; an empty one is wrong.
            Command::SwitchWallet { id, password } => {
                if !self.snapshot.wallets.iter().any(|w| w.id == id) {
                    return changed;
                }
                if password.as_ref().is_some_and(|p| p.expose().is_empty()) {
                    Event::Done {
                        what: "switch",
                        result: Err(CoreError::WrongPassword),
                    }
                } else {
                    for w in &mut self.snapshot.wallets {
                        w.open = w.id == id;
                    }
                    changed = true;
                    if password.is_some() {
                        self.snapshot.vault = VaultState::Unlocked;
                        self.snapshot.wallet = Some(demo_wallet(now()));
                        Event::Done {
                            what: "switch",
                            result: Ok(()),
                        }
                    } else {
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

    #[test]
    fn never_pretends_to_send() {
        let (mut m, tx, rx) = mock();
        m.handle(
            Command::Send {
                from: Source::Core,
                to: "@alice".into(),
                amount: 1,
                password: None,
            },
            &tx,
        );
        assert!(matches!(
            rx.recv().unwrap(),
            Event::Sent(Err(CoreError::NotAvailable(_)))
        ));
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
