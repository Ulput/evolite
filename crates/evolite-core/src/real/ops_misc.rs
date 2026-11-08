//! The small asks: the NFTs shown, the fiat rate, the disk space the data takes.

use super::*;

impl Real {
    /// The NFTs of the shown identity as last found, and an ask for them in the background
    /// when it is due (proofs must be checkable first).
    pub(super) fn shown_nfts(&self, identity: Option<&Identity>) -> (Vec<Nft>, NftsStatus) {
        let owner = identity.map(|i| i.id.clone());
        if let Some(owner) = &owner
            && self.chain.platform_ready()
            && self.nfts.due(owner)
        {
            let name = identity
                .and_then(|i| i.names.iter().find(|n| n.primary).or(i.names.first()))
                .map(|n| format!("@{}", n.label))
                .unwrap_or_else(|| owner.chars().take(8).collect());
            let (chain, nfts, refresh) = (self.chain.clone(), self.nfts.clone(), self.refresher());
            let known = nfts.known_models();
            let owner = owner.clone();
            tokio::spawn(async move {
                let result = match Identifier::from_string(&owner, Encoding::Base58) {
                    Ok(id) => {
                        let read = turn::timed("nft_read", chain.jembe_gifts(id, &known));
                        tokio::time::timeout(nfts::ASK_TIMEOUT, read)
                            .await
                            .unwrap_or_else(|_| Err(nfts::no_answer().into()))
                    }
                    Err(e) => Err(e.to_string()),
                };
                nfts.found(&owner, &name, result);
                refresh();
            });
        }
        self.nfts.shown(owner.as_deref())
    }

    /// "Fiat rate": fetch the price of DASH in `currency` now and every [`rates::EVERY`];
    /// `None` stops and forgets it. A failed fetch keeps the last price and is tried again with
    /// the next round.
    pub(super) fn fiat_rate(&mut self, currency: Option<String>) {
        if let Some(task) = self.fiat_task.take() {
            task.abort();
        }
        let slot = self.fiat.clone();
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

    /// "Disk space": the data folder's bytes, counted again once a minute at most.
    pub(super) fn disk_bytes(&mut self) -> Option<u64> {
        let fresh = self
            .disk
            .is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(60));
        if !fresh {
            self.disk = Some((tokio::time::Instant::now(), dir_size(&self.paths.root)));
        }
        self.disk.map(|(_, bytes)| bytes)
    }

    /// Settings: the Core node to connect to, or none.
    pub(super) async fn core_node(&mut self, address: Option<String>) -> (Option<Event>, bool) {
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
                what: "core-node",
                result,
            }
        };
        (Some(event), true)
    }

    /// Has a stored picture drawn by the renderer.
    pub(super) fn draw_art(
        &mut self,
        art: String,
        side: u32,
        urgent: bool,
    ) -> (Option<Event>, bool) {
        let event = {
            let stored = self.nfts.picture(&art).or_else(|| {
                self.avatars.picture(&art).map(|bytes| crate::art::Stored {
                    bytes,
                    packing: evolite_render::Packing::Plain,
                })
            });
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

    /// Adds an address to the address book.
    pub(super) fn save_address(&mut self, address: String, label: String) -> (Option<Event>, bool) {
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
                what: "address-book",
                result,
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
