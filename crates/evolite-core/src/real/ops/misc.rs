//! The sm
// default-ok: no cursor yet: the search starts from the oldest (a fresh copy)all asks: the NFTs shown, the fiat rate, the disk space the data takes.

use crate::real::*;

/// FEAT-0054 part 2 (the architect, r1 §8d-2): while an identity's own collections are still
/// looked for (BUG-271), its list is «Loading», never «no NFTs»; a search that failed is said
/// as a failure, with «Try again». The lists' states joined as `Nfts::shown` joins them.
pub(in crate::real) fn own_search_held(
    nfts: Vec<Nft>,
    each: Vec<(String, NftsStatus)>,
    own: impl Fn(&str) -> evolite_api::workshop::OwnSearch,
) -> (Vec<Nft>, NftsStatus, Vec<(String, NftsStatus)>) {
    use evolite_api::workshop::OwnSearch;
    let each: Vec<(String, NftsStatus)> = each
        .into_iter()
        .map(|(id, status)| {
            let held = match (own(&id), status) {
                (OwnSearch::Done, s) => s,
                (_, NftsStatus::Failed(why)) => NftsStatus::Failed(why),
                (OwnSearch::Failed, _) => {
                    NftsStatus::Failed(tr!("Could not find your collections").into())
                }
                (OwnSearch::Searching, _) => NftsStatus::Loading,
            };
            (id, held)
        })
        .collect();
    if each.is_empty() {
        return (nfts, NftsStatus::NoIdentity, each);
    }
    let failed = each.iter().find_map(|(_, s)| match s {
        NftsStatus::Failed(why) => Some(why.clone()),
        _ => None,
    });
    let status = match failed {
        Some(why) => NftsStatus::Failed(why),
        None if each.iter().any(|(_, s)| *s == NftsStatus::Loading) => NftsStatus::Loading,
        None => NftsStatus::Ready,
    };
    (nfts, status, each)
}

impl Real {
    /// The NFTs of all the wallet's `identities` together as last found (FEAT-0055), and an ask
    /// in the background for each identity's list when it is due (proofs must be checkable
    /// first).
    pub(in crate::real) fn shown_nfts(
        &self,
        identities: &[IdentityChoice],
    ) -> (Vec<Nft>, NftsStatus, Vec<(String, NftsStatus)>) {
        let owners: Vec<String> = identities.iter().map(|i| i.id.clone()).collect();
        *self
            .wallet_identities
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = owners.clone();
        // The collection's contract is on testnet only: nothing is asked where the network
        // matrix leaves it off (FEAT-0037).
        let here = evolite_api::features::on(
            evolite_api::features::Feature::Collection,
            crate::network::current(),
        );
        // r1 (answer 1): the collections the person chose — any of the wallet's identities' own
        // and those added by id: their pictures are drawn without asking.
        let added = self.added_collections.read(Clone::clone);
        let mut chosen: Vec<String> = identities
            .iter()
            .flat_map(|i| self.chain.workshop_collection_ids(&i.id))
            .collect();
        chosen.extend(added.iter().cloned());
        for identity in identities {
            if !(here && self.chain.platform_ready() && self.nfts.due(&identity.id)) {
                continue;
            }
            let owner = &identity.id;
            let name = identity
                .name
                .as_ref()
                .map(|n| format!("@{n}"))
                .unwrap_or_else(|| owner.chars().take(8).collect());
            let (chain, nfts, refresh) = (self.chain.clone(), self.nfts.clone(), self.refresher());
            // FEAT-0054: the identity's own collections (kept from the Workshop) and the ones
            // added by id; only those of the evolite standard are read.
            let mut collections: Vec<String> = self
                .chain
                .workshop_collection_ids(owner)
                .into_iter()
                .collect();
            for added in &added {
                if !collections.contains(added) {
                    collections.push(added.clone());
                }
            }
            let mut chosen = chosen.clone();
            let owner = owner.clone();
            let search = self.nft_search.clone();
            let journal = self.journal.clone();
            tokio::spawn(async move {
                // TECH-read-deadlines (audit Д9): the limit on a task of its own, not on a
                // `poll` a stuck proof would block (`names::read_within`).
                let read = {
                    let owner = owner.clone();
                    async move {
                        // FEAT-0054 part 2: what came to the identity or it bought, from its
                        // cursor on (a fresh copy from the start), kept by the wallet.
                        let before =
                            // default-ok: no cursor yet: the search starts from the oldest (a fresh copy)
                            search.read(|all| all.get(&owner).cloned().unwrap_or_default());
                        let (found, gifts) = chain.find_received(&owner, &before).await?;
                        // BUG-321: the gifts it got, History's lines — kept before the cursor
                        // moves past them, or a line not kept would not be read again.
                        if !gifts.is_empty() {
                            journal.merge_network(
                                gifts
                                    .iter()
                                    .map(|g| super::workshop::received_entry(g, &owner))
                                    .collect(),
                            )?;
                        }
                        if found != before {
                            let _ = search.update(|all| {
                                all.insert(owner.clone(), found.clone());
                                ((), true)
                            });
                        }
                        for contract in found.contracts() {
                            if !collections.contains(&contract) {
                                collections.push(contract);
                            }
                        }
                        // A purchase is a source the person chose: drawn at once (r1, answer 1).
                        chosen.extend(found.bought.iter().cloned());
                        turn::timed(
                            "nft_read",
                            chain.standard_nfts(&owner, &name, &collections, &chosen),
                        )
                        .await
                    }
                };
                let result = super::names::read_within(nfts::ASK_TIMEOUT, read)
                    .await
                    .map_err(|e| match super::names::timed_out(&e) {
                        true => nfts::no_answer().into(),
                        false => e,
                    });
                if result.is_ok() {
                    crate::diag::fresh("nfts", "network");
                }
                nfts.found(&owner, result);
                refresh();
            });
        }
        let (nfts, _, each) = self.nfts.shown(&owners);
        own_search_held(nfts, each, |id| self.own_search_of(id))
    }

    /// FEAT-0054 «Add a collection by id»: the contract read with its proof and kept only when
    /// it is an NFT collection of the evolite standard; the identities' lists are asked again.
    pub(in crate::real) async fn add_collection(&self, id: &str) -> Result<(), CoreError> {
        let id = id.trim().to_owned();
        // r1 Should 2: not where the network matrix leaves the Collection off (FEAT-0037).
        if !evolite_api::features::on(
            evolite_api::features::Feature::Collection,
            crate::network::current(),
        ) {
            return Err(CoreError::NotAvailable(
                tr!("NFTs of evolite collections: testnet only for now").into(),
            ));
        }
        if Identifier::from_string(&id, Encoding::Base58).is_err() {
            return Err(CoreError::NotAvailable(
                tr!("This is not a contract id: 44 letters and digits").into(),
            ));
        }
        // r1 Should 3: a read that failed is said in words; the SDK's text goes to the journal.
        let standard = self.chain.is_standard_collection(&id).await.map_err(|e| {
            crate::diag::platform_failed("collection_check", &e);
            CoreError::NotAvailable(tr!("Could not check the contract — try again").into())
        })?;
        if !standard {
            return Err(CoreError::NotAvailable(
                tr!("This contract is not an NFT collection of the evolite standard").into(),
            ));
        }
        self.added_collections
            .update(|all| {
                let new = !all.contains(&id);
                if new {
                    all.push(id.clone());
                }
                ((), new)
            })
            .map_err(|e| CoreError::NotAvailable(e.to_string()))?;
        self.nfts.ask_now();
        Ok(())
    }

    /// FEAT-0054 r1: «Always show from …» — `issuer`'s pictures drawn without asking, kept.
    pub(in crate::real) fn trust_issuer(&self, issuer: &str) -> Result<(), CoreError> {
        let issuer = issuer.trim().to_owned();
        if issuer.is_empty() {
            return Ok(());
        }
        self.trusted_issuers
            .update(|all| {
                let new = !all.contains(&issuer);
                if new {
                    all.push(issuer.clone());
                }
                ((), new)
            })
            .map_err(|e| CoreError::NotAvailable(e.to_string()))
    }

    /// FEAT-0054: the collection `id` added by hand no longer shown.
    pub(in crate::real) fn remove_collection(&self, id: &str) -> Result<(), CoreError> {
        self.added_collections
            .update(|all| {
                let before = all.len();
                all.retain(|c| c != id);
                ((), all.len() != before)
            })
            .map_err(|e| CoreError::NotAvailable(e.to_string()))?;
        self.nfts.ask_now();
        Ok(())
    }

    /// "Fiat rate": fetch the price of DASH in `currency` now and every [`rates::EVERY`];
    /// `None` stops and forgets it. A failed fetch keeps the last price and is tried again with
    /// the next round.
    pub(in crate::real) fn fiat_rate(&mut self, currency: Option<String>) {
        if let Some(task) = self.fiat_task.take() {
            task.abort();
        }
        let slot = self.fiat.clone();
        self.fiat_currency = currency.clone();
        let Some(currency) = currency else {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return;
        };
        if slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|f| f.currency != currency)
        {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        let (dirty, wake) = (self.dirty.clone(), self.wake.clone());
        self.fiat_task = Some(tokio::spawn(async move {
            loop {
                let asked = currency.clone();
                if let Ok(Ok(price)) =
                    tokio::task::spawn_blocking(move || rates::fetch(&asked)).await
                {
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(FiatRate {
                        currency: currency.clone(),
                        price,
                        at: now(),
                    });
                    dirty.store(true, Ordering::Relaxed);
                    wake();
                }
                tokio::time::sleep(rates::EVERY).await;
            }
        }));
    }

    /// Р1, Settings › Privacy › «Check for a new version»: on, the release's signed `latest.json`
    /// is read once a day — the first time only once a wallet was open in this run (review r1),
    /// never at a bare start — and what a signed, not older file says goes into the snapshot. Off,
    /// the task stops and what was learnt is forgotten. Each check goes to the journal by its
    /// result and the file's date.
    pub(in crate::real) fn check_update(&mut self, on: bool) {
        use evolite_local::update;
        if let Some(task) = self.update_task.take() {
            task.abort();
        }
        let slot = self.update.clone();
        if !on {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return;
        }
        let (dirty, wake) = (self.dirty.clone(), self.wake.clone());
        let opened = self.wallet_opened.clone();
        self.update_task = Some(tokio::spawn(async move {
            while !opened.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            // The newest date believed in this run: an older signed file is a replay.
            let mut seen: Option<String> = None;
            loop {
                let last = seen.clone();
                let checked =
                    tokio::task::spawn_blocking(move || update::check(last.as_deref())).await;
                match checked {
                    Ok(Ok(latest)) => {
                        let newer = update::is_newer(&latest.version, env!("CARGO_PKG_VERSION"));
                        crate::diag::record(crate::diag::Record::UpdateCheck {
                            result: if newer { "newer" } else { "same" },
                            date: Some(latest.date.clone()),
                        });
                        seen = Some(latest.date.clone());
                        *slot.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(evolite_api::UpdateInfo {
                                version: latest.version,
                                newer,
                                critical: latest.critical,
                                date: latest.date,
                                checked_at: now(),
                                notes_url: latest.notes_url,
                                verify_url: latest.verify_url,
                            });
                        dirty.store(true, Ordering::Relaxed);
                        wake();
                    }
                    Ok(Err(refused)) => crate::diag::record(crate::diag::Record::UpdateCheck {
                        result: refused.kind(),
                        date: None,
                    }),
                    Err(_) => {}
                }
                tokio::time::sleep(update::EVERY).await;
            }
        }));
    }

    /// "Disk space": the data folder's bytes, counted again once a minute at most.
    pub(in crate::real) fn disk_bytes(&mut self) -> Option<u64> {
        let fresh = self
            .disk
            .is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(60));
        if !fresh {
            self.disk = Some((tokio::time::Instant::now(), dir_size(&self.paths.root)));
        }
        self.disk.map(|(_, bytes)| bytes)
    }

    /// Settings: the Core node to connect to, or none.
    pub(in crate::real) async fn core_node(
        &mut self,
        address: Option<String>,
    ) -> (Option<Event>, bool) {
        let event = {
            let result = match address.as_deref().map(node::normalize).transpose() {
                Err(why) => Err(CoreError::NotAvailable(why)),
                Ok(address) => {
                    let found = match &address {
                        Some(a) => node::resolve(a).await.map(|_| ()),
                        None => Ok(()),
                    };
                    match found {
                        Err(why) => Err(CoreError::NotAvailable(why)),
                        Ok(()) => node::save(&self.paths.chain.core_node, address.as_deref())
                            .map_err(|e| CoreError::Failed(e.to_string())),
                    }
                }
            };
            Event::Done {
                what: DoneWhat::CoreNode,
                result,
                ask: None,
            }
        };
        (Some(event), true)
    }

    /// Has a stored picture drawn by the renderer.
    pub(in crate::real) fn draw_art(
        &mut self,
        art: String,
        side: u32,
        urgent: bool,
    ) -> (Option<Event>, bool) {
        let event = {
            let stored = self.stored_art(&art);
            match (stored, &self.artist) {
                (Some(stored), Some(artist)) => {
                    artist.draw(art, side, urgent, stored);
                    return (None, false);
                }
                _ => Event::Art {
                    art,
                    side,
                    partial: false,
                    result: Err(CoreError::NotAvailable(tr!("No picture").into())),
                },
            }
        };
        (Some(event), true)
    }

    /// The bytes of the picture `art` names, wherever this wallet has them.
    pub(in crate::real) fn stored_art(&self, art: &str) -> Option<crate::art::Stored> {
        self.avatars
            .picture(art)
            .map(|bytes| crate::art::Stored {
                bytes,
                packing: evolite_render::Packing::Plain,
            })
            // BUG-269: a Workshop model's picture as read from Platform, on any copy — and,
            // FEAT-0054, every NFT's of the Collection.
            .or_else(|| {
                self.chain
                    .workshop_picture(art)
                    .map(|(bytes, deflated)| crate::art::Stored {
                        bytes,
                        packing: if deflated {
                            evolite_render::Packing::RawDeflate
                        } else {
                            evolite_render::Packing::Plain
                        },
                    })
            })
            // FEAT-0038 W5: the file chosen for a new model and its copy.
            .or_else(|| self.artist.as_ref().and_then(|a| a.local(art)))
    }

    /// Adds an address to the address book.
    pub(in crate::real) fn save_address(
        &mut self,
        address: String,
        label: String,
    ) -> (Option<Event>, bool) {
        let event = {
            // Only a Core testnet address: a name pays as a contact, not from the book.
            let result = payee(&address)
                .and_then(|a| {
                    a.require_network(crate::network::dash())
                        .map_err(|_| CoreError::BadAddress)
                })
                .and_then(|a| {
                    self.address_book
                        .save(&a.to_string(), &label)
                        .map_err(CoreError::NotAvailable)
                });
            self.dirty.store(true, Ordering::Relaxed);
            Event::Done {
                what: DoneWhat::AddressBook,
                result,
                ask: None,
            }
        };
        (Some(event), true)
    }
}

/// Bytes of the files under `dir`, however deep; what cannot be read counts as nothing.
fn dir_size(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(t) if t.is_dir() => dir_size(&entry.path()),
            Ok(t) if t.is_file() => entry.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}
