//! What the interface is shown: the snapshot of the open wallet, built from the chain, the store and the journal.

use super::*;
use evolite_chain::lock_status;
use key_wallet::managed_account::transaction_record::TransactionRecord;
use platform_wallet::{ManagedIdentity, PlatformWallet};

/// An identity of this wallet by its id, as a card names it.
fn identity_ref(identities: &[IdentityChoice], id: &str) -> Option<IdentityRef> {
    identities.iter().find(|c| c.id == id).map(|c| IdentityRef {
        id: c.id.clone(),
        name: c.name.clone(),
    })
}

/// Whether a lock's line names this identity ([`identity::lock_line`]: `@name`, or its id).
fn lock_names(c: &IdentityChoice, shown: &str) -> bool {
    match &c.name {
        Some(name) => shown.strip_prefix('@') == Some(name.as_str()),
        None => shown == c.id,
    }
}

/// The identity the wallet works as now (base58): the one a journal entry is written for (FR-5).
pub(in crate::real) async fn acting_identity(wallet: &PlatformWallet) -> Option<String> {
    let state = wallet.state().await;
    let id = wallet.wallet_id();
    let mine: Vec<_> = state
        .identity_manager
        .managed_identities()
        .filter(|m| m.wallet_id == Some(id) && m.identity_index.is_some())
        .collect();
    let slot = chain::shown_slot(id, mine.iter().filter_map(|m| m.identity_index));
    mine.into_iter()
        .find(|m| m.identity_index == slot)
        .map(|m| m.identity.id().to_string(Encoding::Base58))
}

/// The private pool's own lines get a card of their own (FR-3, frame 7): their direction is
/// their kind. A payment out of the pool does not keep its recipient, and says so once (FR-6).
fn describe_pool_lines(history: &mut [Activity]) {
    for act in history.iter_mut().filter(|a| a.detail.is_none()) {
        let incomplete = match act.kind {
            ActivityKind::PoolSent => true,
            ActivityKind::PoolReceived | ActivityKind::Shielded { .. } => false,
            _ => continue,
        };
        if act.chain == evolite_api::Chain::Platform {
            act.detail = Some(Box::new(Detail {
                incomplete,
                ..Detail::default()
            }));
        }
    }
}

/// Puts on an asset lock's line what [`identity::lock_line`] found: whom it was for, whether
/// Platform took it, and for a card how many credits a top-up from this computer added (FR-3)
/// and which identity to open (AC-T5). The amount is asked of the status just set, not of the
/// `Unknown` the line was built with.
pub(super) fn say_the_lock(
    act: &mut Activity,
    (who, made, taken): (String, bool, Credits),
    top_ups: &HashMap<String, Duffs>,
    identities: &[IdentityChoice],
) {
    let ActivityKind::TopUp {
        identity,
        creates,
        credits,
    } = &mut act.kind
    else {
        return;
    };
    if identity.is_empty() {
        *identity = who;
    }
    (*creates, *credits) = (made, taken);
    let Some(detail) = act.detail.as_deref_mut() else {
        return;
    };
    if *credits == Credits::Added {
        detail.credits_added = act.txid.as_deref().and_then(|t| top_ups.get(t)).copied();
    }
    if let Some(c) = identities.iter().find(|c| lock_names(c, identity)) {
        detail.identity = identity_ref(identities, &c.id);
    }
}

/// The asset locks platform-wallet tracks, by txid.
async fn tracked_locks(wallet: &PlatformWallet) -> HashMap<String, identity::Tracked> {
    turn::timed("tracked_locks", wallet.asset_locks().list_tracked_locks())
        .await
        .into_iter()
        .map(|l| {
            (
                l.out_point.txid.to_string(),
                identity::Tracked {
                    funding: l.funding_type,
                    slot: l.identity_index,
                    consumed: l.status.is_consumed(),
                },
            )
        })
        .collect()
}

/// The pool as the snapshot shows it.
fn shown_pool(
    pool: Option<pool::PoolView>,
    unfinished_shield: Option<Duffs>,
    connecting: bool,
) -> Pool {
    match pool {
        Some(pool) => Pool {
            notes: pool.notes,
            checked_at: pool.checked_at,
            address: Some(pool.address),
            unfinished_shield,
            connecting: false,
            operations: pool.operations,
        },
        None => Pool {
            notes: 0,
            checked_at: None,
            address: None,
            unfinished_shield,
            connecting,
            operations: Vec::new(),
        },
    }
}

/// The identity the person works as, its credits and its contacts.
type ShownIdentity = (Identity, Duffs, Vec<Contact>);

/// What one read of a wallet's own state gives its snapshot: empty while the wallet is not open.
#[derive(Default)]
struct WalletRead {
    balance: Option<Duffs>,
    /// The coins, and the addresses holding them (the overview counts those).
    coins: usize,
    addresses: usize,
    /// The coins' values for the fee of a payment (BUG-55).
    coin_values: Vec<u64>,
    /// What of the balance no payment can sign for yet (BUG-142).
    unsignable: Duffs,
    /// The contacts' receiving accounts, by [`contact_scans::ContactScans::key`] (BUG-150).
    contact_accounts: Vec<String>,
    history: Vec<Activity>,
    shown: Option<ShownIdentity>,
    platform: Option<(String, u64)>,
    unfinished: Option<UnfinishedIdentity>,
    unfinished_top_up: Option<UnfinishedTopUp>,
    lost_block: Option<mined::Lost>,
    /// How far the wallet's filters are: back where a rescan was asked, it has finished.
    synced: u32,
    identities: Vec<IdentityChoice>,
    /// What each asset lock in the history is for, by the account it pays (BUG-57).
    lock_fors: HashMap<String, identity::LockFor>,
    /// Every identity's credits, the one worked as among them (BUG-56).
    all_credits: Duffs,
    /// The wallet's own Core addresses (FEAT-0009: a withdrawal to one is a move between own).
    own_core: Vec<String>,
    /// The CoinJoin account's coins, apart from Core (FEAT-0030 AC-T3).
    mixed: Option<Mixed>,
}

impl Real {
    pub(super) async fn snapshot(&mut self) -> Snapshot {
        let status = self.chain.status();
        let vault = match (&self.unlocked, &self.saved) {
            (Some(_), _) => VaultState::Unlocked,
            (None, Some(_)) => VaultState::Locked,
            (None, None) => VaultState::NoWallet,
        };
        let wallet = match &self.unlocked {
            Some(open) => Some(self.wallet(open.id, open.meta.clone()).await),
            None => None,
        };
        // DAPI nodes the SDK may ask now (not banned for a bad answer).
        // Asked again every few seconds at most (the masternode list is large).
        let fresh = self
            .nodes
            .as_ref()
            .is_some_and(|(at, _, _)| at.elapsed() < Duration::from_secs(5));
        if !fresh {
            let core = turn::timed("core_peers", self.chain.core_peers()).await;
            let dapi = turn::timed("dapi_nodes", self.chain.dapi_nodes()).await;
            self.nodes = Some((tokio::time::Instant::now(), core, dapi));
        }
        let (core_nodes, dapi_nodes) = self
            .nodes
            .as_ref()
            .map(|(_, c, d)| (c.clone(), d.clone()))
            .unwrap_or_default();
        let platform_nodes = dapi_nodes
            .iter()
            .filter(|n| n.banned_until.is_none())
            .count() as u32;
        Snapshot {
            network: crate::network::current(),
            sync: SyncInfo {
                disk_bytes: turn::during("disk_bytes", || self.disk_bytes()),
                platform_nodes,
                core_nodes,
                dapi_nodes,
                core_node_next: {
                    let saved =
                        turn::during("core_node_file", || node::load(&self.paths.chain.core_node));
                    // Empty: back to DNS seeds at the next start.
                    (saved != status.core_node).then(|| saved.unwrap_or_default())
                },
                ..self.shown_sync(&status)
            },
            vault,
            wallet,
            demo: false,
            elsewhere: false,
            fiat: self.fiat.lock().map(|f| f.clone()).unwrap_or_default(),
            wallets: self.wallet_entries(),
            adding_wallet: self.adding_from.is_some(),
        }
    }

    /// The sync as the screens show it. A rescan for the contacts' receiving accounts (BUG-150) takes
    /// the filters back to the birth in the background: the screens go on saying «Done», not
    /// «syncing» from the start again (the architect). A payment waits for it all the same.
    fn shown_sync(&self, status: &sync::SyncStatus) -> SyncInfo {
        let mut shown = sync_info(status, self.unlocked.is_some());
        let rescanning = self
            .unlocked
            .as_ref()
            .is_some_and(|o| !OwedRescan::contacts_under_way(&self.rescan_owed, &o.id).is_empty());
        if rescanning && status.phase != sync::Phase::Error {
            shown.stage = SyncStage::Done;
            shown.progress = None;
        }
        shown
    }

    /// The wallets of this computer for the switcher, this core's marked open.
    pub(super) fn wallet_entries(&mut self) -> Vec<WalletEntry> {
        self.index
            .wallets
            .iter()
            .map(|r| WalletEntry {
                id: r.id.clone(),
                name: r.name.clone(),
                color: r.color,
                origin: match r.origin {
                    wallets::Origin::Created => WalletOrigin::Created,
                    wallets::Origin::Restored => WalletOrigin::Restored,
                    wallets::Origin::DashPay => WalletOrigin::DashPay,
                },
                created_at: r.created_at,
                last_opened: r.last_opened,
                open: r.id == self.wallet,
                wrong_tries: self.attempts.failures(&r.id),
                wait_until: self
                    .attempts
                    .check(&r.id, wall_secs())
                    .map(|t| t as Timestamp),
            })
            .collect()
    }

    /// The other wallet of this computer the phrase already is (same `WalletId`), if any: a
    /// phrase is one wallet here, not two folders syncing the same keys.
    pub(super) fn same_wallet(&self, phrase: &str) -> Option<String> {
        let id = chain::wallet_id_of(phrase).ok()?;
        let id = hex(&id);
        self.index
            .wallets
            .iter()
            .filter(|r| r.id != self.wallet)
            .find(|r| {
                vault::load(&wallets::wallet_dir(&self.paths.root, &r.id).join("wallet.json"))
                    .ok()
                    .flatten()
                    .is_some_and(|f| f.meta.wallet_id == id)
            })
            .map(|r| r.name.clone())
    }

    pub(super) async fn wallet(&mut self, id: WalletId, meta: VaultMeta) -> Wallet {
        // Read before the coins are counted, and again after ([`count_checked`]).
        let status_before = self.chain.status();
        let WalletRead {
            balance,
            coins,
            addresses,
            coin_values,
            unsignable,
            contact_accounts,
            mut history,
            shown,
            platform,
            unfinished,
            unfinished_top_up,
            lost_block,
            synced,
            identities,
            lock_fors,
            all_credits,
            own_core,
            mixed,
        } = self.read_wallet(id).await;
        let (platform_address, platform_credits) = platform.unwrap_or_default();
        let (pool, unfinished_shield, connecting_pool) = self.read_pool(id).await;
        let mut fees = turn::timed("fees_now", self.fees_now(coins)).await;
        fees.core_coins = convert::core_coins(coin_values);
        fees.core_unsignable = unsignable;
        // The pool's own operations are in the history too (only the owner sees them), and a
        // Core asset lock made for the pool says so instead of "Transfer to Platform".
        if let Some(pool) = &pool {
            history.extend(pool.operations.iter().cloned());
        } else {
            // Not connected yet (a start): what the file keeps, not nothing (BUG-103).
            history.extend(self.pool_history.kept());
        }
        describe_pool_lines(&mut history);
        // Sends on Platform shown from "Sign" until their own lines are here (BUG-12).
        let mut history = self
            .platform_sending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .apply(history, now());
        self.say_what_locks_are(
            id,
            &mut history,
            &identities,
            &lock_fors,
            unfinished.as_ref(),
            unfinished_top_up.as_ref(),
        )
        .await;
        let credits = shown.as_ref().map_or(0, |(_, c, _)| *c);
        let mut contacts = shown
            .as_ref()
            .map(|(_, _, contacts)| contacts.clone())
            .unwrap_or_default();
        let unnamed = contacts
            .iter()
            .filter(|c| c.name.is_none())
            .map(|c| c.identity_id.clone())
            .collect();
        let (credits_read, credits_checked) = {
            let search = self
                .identity_search
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            (
                credits_known(!identities.is_empty(), &search),
                credits_checked(
                    self.credits_checked.load(Ordering::Relaxed),
                    !identities.is_empty(),
                    &search,
                ),
            )
        };
        self.ask_names(unnamed);
        self.contacts = contacts.clone();
        let (nfts, nfts_status) = self.shown_nfts(shown.as_ref().map(|(identity, _, _)| identity));
        let apps = app_jobs::shown_apps(
            &self.apps.all(),
            shown.as_ref().map(|(identity, _, _)| identity.id.as_str()),
            &self.app_reads.lock().unwrap_or_else(|e| e.into_inner()),
        );
        self.name_contact_lines(id, &mut history, &mut contacts)
            .await;
        convert::settle(&mut history, self.chain.status().chainlock_height);
        self.mempool.mark_stale(
            &mut history,
            self.chain.status().headers.map_or(0, |h| h.height),
            now(),
        );
        self.settle_rescans(
            id,
            meta.birth_height,
            synced,
            lost_block,
            &contact_accounts,
            &mut history,
        )
        .await;
        let notes_error = self.put_notes_on(&mut history).await;
        history.sort_by_key(|a| std::cmp::Reverse(a.at));
        // Only a checked balance is shown: while the filters are not at the tip (a start, a
        // rewound pass, a rescan) the last checked one with its time, or none (BUG-77).
        let (core, core_as_of) = self.core_balance.shown(
            balance.unwrap_or(0),
            count_checked(balance.is_some(), &status_before, &self.chain.status()),
            now(),
        );
        let mut shown_wallet = Wallet {
            password_each_send: self.password_each_send,
            notes_error,
            tokens: self.shown_tokens(
                shown.as_ref().map(|(identity, _, _)| identity.id.as_str()),
                identities.iter().map(|c| c.id.clone()).collect(),
            ),
            platform_inputs: self.shown_address_inputs.clone(),
            balances: Balances {
                core,
                core_as_of,
                core_addresses: addresses as u32,
                credits,
                all_credits,
                platform_address: identity::credits_as_duffs(platform_credits),
                shielded: pool
                    .as_ref()
                    .map_or(0, |p| identity::credits_as_duffs(p.credits)),
                credits_read,
                credits_checked,
                platform_address_read: platform_address_known(
                    self.chain.status().platform_address_read,
                    platform_credits,
                ),
                mixed,
            },
            activity: history,
            core_address: self
                .receive_address
                .as_ref()
                .map(|(a, _)| a.clone())
                .unwrap_or_default(),
            platform_address,
            identity: shown.map(|(identity, _, _)| identity),
            pool: shown_pool(pool, unfinished_shield, connecting_pool),
            apps,
            nfts,
            nfts_status,
            contacts,
            unfinished_identity: unfinished,
            unfinished_top_up,
            identities,
            address_book: self
                .address_book
                .entries()
                .into_iter()
                .map(|e| SavedAddress {
                    address: e.address,
                    label: e.label,
                })
                .collect(),
            identity_search: self
                .identity_search
                .lock()
                .map(|s| s.clone())
                .unwrap_or_default(),
            fees,
            phrase_words: meta.word_count as usize,
            created_at: meta.created_at as Timestamp,
        };
        // Which balances each line changes and which way, with what is the wallet's own
        // (FEAT-0009 R-1, R-2).
        crate::layers::settle_wallet(&mut shown_wallet, &own_core);
        shown_wallet
    }

    /// Everything a snapshot takes from the wallet's own state, under one read of it.
    async fn read_wallet(&mut self, id: WalletId) -> WalletRead {
        let found = turn::timed("wallet_lookup", self.chain.wallet(&id)).await;
        let Some(wallet) = found else {
            return WalletRead::default();
        };
        let records = {
            let state = turn::timed("state_records", wallet.state()).await;
            state.core_wallet.transaction_history().len()
        };
        let own_core = turn::timed("state_addresses", wallet.state())
            .await
            .core_wallet
            .monitored_addresses()
            .iter()
            .map(ToString::to_string)
            .collect();
        self.refresh_receive_address(&wallet, records).await;
        let platform = turn::timed("platform_address", self.chain.platform_address(&wallet))
            .await
            .ok();
        // What a payment from the addresses plans with on the form (FEAT-0015).
        self.shown_address_inputs =
            turn::timed("address_inputs", self.chain.address_inputs(&wallet)).await;
        let unfinished =
            turn::timed("unfinished_identity", self.unfinished_identity(&wallet)).await;
        let unfinished_top_up =
            turn::timed("unfinished_top_up", self.unfinished_top_up(&wallet)).await;
        let live_count = {
            let state = turn::timed("state_count", wallet.state()).await;
            state.core_wallet.transaction_history().len()
        };
        turn::timed(
            "refresh_stored_history",
            self.refresh_stored_history(id, live_count),
        )
        .await;
        let state = turn::timed("state_history", wallet.state()).await;
        let _building = turn::begin("history_build");
        let synced = state.core_wallet.synced_height();
        let mut all = self.merged_records(&state.core_wallet);
        let locks: Vec<_> = all
            .iter()
            .filter(|r| {
                r.transaction_type
                    == key_wallet::transaction_checking::transaction_router::TransactionType::AssetLock
            })
            .map(|r| (r.txid.to_string(), &r.transaction))
            .collect();
        let lock_fors = identity::lock_fors(
            &state.core_wallet.accounts,
            &mut self.top_ups,
            id,
            self.unlocked.as_ref().and_then(|o| o.keys.as_ref()),
            state
                .identity_manager
                .managed_identities()
                .filter(|m| m.wallet_id == Some(id))
                .filter_map(|m| m.identity_index),
            &locks,
        );
        // One the wallet took back out of its block is shown in it, and the filters are
        // scanned again from there so the wallet and the store get it back (BUG-64).
        let lost_block = self
            .mined
            .recall(&mut all, self.chain.status().chainlock_height);
        let unlocks = convert::asset_unlocks(&all);
        let coinjoin = convert::coinjoin_addresses(&state.core_wallet);
        let records = self.activity_lines(&all, &coinjoin);
        let paid_at: HashMap<String, Timestamp> =
            records.iter().map(|(t, a)| (t.clone(), a.at)).collect();
        // The interface shows one identity: the first slot of the phrase, with its
        // DashPay profile and contacts.
        let names = self.names.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mine: Vec<_> = state
            .identity_manager
            .managed_identities()
            .filter(|m| m.wallet_id == Some(id) && m.identity_index.is_some())
            .collect();
        let mut identities: Vec<_> = mine
            .iter()
            .map(|m| IdentityChoice {
                id: m.identity.id().to_string(Encoding::Base58),
                name: identity::to_api(m).name().map(str::to_owned),
                slot: m.identity_index.unwrap_or_default(),
            })
            .collect();
        identities.sort_by_key(|c| c.slot);
        let all_credits = mine
            .iter()
            .map(|m| identity::credits_as_duffs(m.identity.balance()))
            .sum();
        let slot = chain::shown_slot(id, mine.iter().filter_map(|m| m.identity_index));
        // The identity the person works as (the first one until they choose).
        let first = mine.iter().copied().find(|m| m.identity_index == slot);
        let shown = first.map(|m| self.shown_identity(m, &names, &paid_at));
        // Payments to and from contacts say who; this run's payments too, and they
        // show until the history has them.
        let records = match first {
            Some(m) => dashpay::name_payments(m, &names, records),
            None => records,
        };
        let mut history = self
            .outgoing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .apply(records);
        let answered = answered_requests(&mine);
        let own_addresses = convert::own_platform_addresses(&state.core_wallet);
        self.add_platform_lines(
            &mut history,
            (&identities, &own_addresses),
            &names,
            &answered,
        );
        platform_history::link_payouts(&mut history, &self.journal.entries(), &unlocks, |id| {
            identity_ref(&identities, id)
        });
        // BUG-150: the coins by kind once the first filter pass is done (numbers only).
        if self.coins_after_pass != Some(id) && self.chain.status().phase == sync::Phase::Synced {
            crate::diag::record(convert::coins_record("after_filters", &state.core_wallet));
            self.coins_after_pass = Some(id);
        }
        // Coins, and the addresses holding them (the overview counts those).
        let utxos = convert::coins_once(&state.core_wallet);
        let holding: std::collections::HashSet<_> =
            utxos.iter().map(|u| u.address.clone()).collect();
        WalletRead {
            // Each coin once, not platform-wallet's sum of the accounts (BUG-77).
            balance: Some(convert::core_balance(&state.core_wallet)),
            coins: utxos.len(),
            addresses: holding.len(),
            coin_values: utxos.iter().map(|u| u.value()).collect(),
            unsignable: convert::unsignable(&state.core_wallet),
            contact_accounts: state
                .core_wallet
                .accounts
                .dashpay_receival_accounts
                .keys()
                .map(|k| {
                    contact_scans::ContactScans::key(
                        &k.user_identity_id,
                        &k.friend_identity_id,
                        k.index,
                    )
                })
                .collect(),
            history,
            shown,
            platform,
            unfinished,
            unfinished_top_up,
            lost_block,
            synced,
            identities,
            lock_fors,
            all_credits,
            own_core,
            // Rounds counted in one pass over the history (FEAT-0030 AC-T8).
            mixed: convert::mixed_balance(&convert::mixed_coins(&state.core_wallet, |txid| {
                self.chain.instant_lock_verified(txid)
            })),
        }
    }

    /// The address Receive shows: one for every payment, or a new one after each if the person
    /// asked for that.
    async fn refresh_receive_address(&mut self, wallet: &PlatformWallet, records: usize) {
        if !self.fresh_receive {
            // One address for every payment: the first of the wallet's receive chain
            // (m/44'/1'/0'/0/0), the same after a restart or a restore.
            let state = turn::timed("state_receive", wallet.state()).await;
            self.receive_address = first_receive_address(&state.core_wallet).map(|a| (a, records));
        } else if self
            .receive_address
            .as_ref()
            .is_none_or(|(_, at)| *at != records)
        {
            self.receive_address = turn::timed(
                "next_receive_address",
                wallet.core().next_receive_address_for_account(0),
            )
            .await
            .ok()
            .map(|a| (a.to_string(), records));
        }
    }

    /// The store's records first: one per transaction, as platform-wallet folds the accounts it
    /// touches. Then this run's records the store has not got yet, one per transaction too, each
    /// input and output once over the accounts (BUG-85), so an asset lock — which touches two
    /// accounts — is one line, not two.
    fn merged_records(
        &self,
        info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    ) -> Vec<TransactionRecord> {
        let stored_ids: std::collections::HashSet<_> =
            self.stored_history.iter().map(|r| r.txid).collect();
        let fresh = convert::one_per_transaction(
            info.transaction_history()
                .into_iter()
                .filter(|r| !stored_ids.contains(&r.txid)),
        );
        // A spend recorded before the coin it spends (a restore's rescan) gets its
        // input back from our own outputs.
        let mut all: Vec<_> = self.stored_history.iter().cloned().chain(fresh).collect();
        convert::attribute_late_inputs(&mut all);
        all
    }

    /// One history line per record, by its txid.
    fn activity_lines(
        &self,
        all: &[TransactionRecord],
        coinjoin: &std::collections::HashSet<key_wallet::dashcore::Address>,
    ) -> Vec<(String, Activity)> {
        let now = now();
        let tip = self.chain.status().headers.map_or(0, |h| h.height);
        all.iter()
            .map(|r| {
                let txid = r.txid.to_string();
                // A block gives its own time: only what is out of one is kept (each
                // new one is a write of the file).
                let seen = match r.context.block_info() {
                    Some(_) => now,
                    None => self.mempool.first_seen(&txid, now, tip),
                };
                let verified = self.chain.instant_lock_verified(&r.txid);
                let mut line = activity(r, seen, verified);
                // FEAT-0030 A3: mixing rounds, the preparation, payments from «Mixed».
                convert::mark_mixing(&mut line, r, coinjoin);
                (txid, line)
            })
            .collect()
    }

    /// The identity the person works as, with its profile and contacts.
    fn shown_identity(
        &self,
        m: &ManagedIdentity,
        names: &HashMap<String, Option<String>>,
        paid_at: &HashMap<String, Timestamp>,
    ) -> ShownIdentity {
        let avatars = self.avatars.clone();
        let refresh = self.refresher();
        let avatar = |owner: &Identifier, p: &platform_wallet::DashPayProfile| {
            avatars.want(
                &dashpay::base58(owner),
                p.avatar_url.as_deref(),
                p.avatar_hash,
                refresh.clone(),
            )
        };
        let mut api = identity::to_api(m);
        api.profile = m
            .dashpay()
            .profile
            .as_ref()
            .map(|p| dashpay::profile_of(p, avatar(&m.identity.id(), p)));
        (
            api,
            identity::credits_as_duffs(m.identity.balance()),
            dashpay::contacts_of(m, names, avatar, |txid| paid_at.get(txid).copied())
                .into_iter()
                // Hidden before a resync: the new store does not know it.
                .filter(|c| {
                    c.state != ContactState::Incoming || !self.hidden.contains(&c.identity_id)
                })
                .collect::<Vec<_>>(),
        )
    }

    /// What the wallet did on Platform, which Core does not see. A request names both sides as
    /// the wallet knows them now: our identities by their names, contacts by theirs (BUG-69),
    /// and which identity of ours it went from or came to when there are several (BUG-58); one
    /// hidden here before a resync stays hidden.
    fn add_platform_lines(
        &self,
        history: &mut Vec<Activity>,
        (identities, own_addresses): (&[IdentityChoice], &std::collections::HashSet<String>),
        names: &HashMap<String, Option<String>>,
        answered: &HashMap<String, std::collections::HashSet<String>>,
    ) {
        let own: std::collections::HashMap<&str, String> = identities
            .iter()
            .filter_map(|c| Some((c.id.as_str(), format!("@{}", c.name.as_ref()?))))
            .collect();
        let named = |id: &str| {
            own.get(id).cloned().or_else(|| {
                names
                    .get(id)
                    .cloned()
                    .flatten()
                    .map(|label| format!("@{label}"))
            })
        };
        history.extend(self.journal.entries().iter().map(|entry| {
            let mut line = journal::activity(entry);
            // To one of the wallet's own Platform addresses: only the fee left (QA pass 21).
            if let ActivityKind::AddressSent { to } = &line.kind
                && convert::is_own_platform_address(own_addresses, to)
            {
                journal::between_own_addresses(&mut line);
            }
            platform_history::name_request_sides(
                &mut line.kind,
                entry,
                identities.len() > 1,
                named,
            );
            // The card names the identity as the wallet knows it now (AC-T5).
            if let Some(detail) = line.detail.as_deref_mut()
                && let Some(known) = detail
                    .identity
                    .as_ref()
                    .and_then(|i| identity_ref(identities, &i.id))
            {
                detail.identity = Some(known);
            }
            if let ActivityKind::ContactRequestIn { state, .. } = &mut line.kind {
                *state = settle_incoming(
                    *state,
                    entry.key.as_deref(),
                    |f| self.hidden.contains(f),
                    |ours, f| {
                        answered
                            .get(ours)
                            .is_some_and(|contacts| contacts.contains(f))
                    },
                );
            }
            line
        }));
    }

    /// The shielded pool, once connected (local reads, no network): the view, the shield left
    /// unfinished, and whether it was asked to connect for this wallet and is not ready yet.
    async fn read_pool(&self, id: WalletId) -> (Option<pool::PoolView>, Option<Duffs>, bool) {
        let found = turn::timed("wallet_lookup_pool", self.chain.wallet(&id)).await;
        let (pool, unfinished_shield) = match found {
            Some(wallet) => (
                turn::timed("pool_view", self.chain.pool_view(&wallet)).await,
                turn::timed(
                    "unfinished_pool_shield",
                    self.chain.unfinished_pool_shield(&wallet),
                )
                .await
                .map(|duffs| duffs as Duffs),
            ),
            None => (None, None),
        };
        // Asked to connect for this wallet and not ready yet.
        let connecting_pool = pool.is_none()
            && *self.pool_wallet.lock().unwrap_or_else(|e| e.into_inner()) == Some(id);
        // What the scan lost after a restart (a top-up from the pool as 0), put back.
        let pool = pool.map(|mut p| {
            p.operations = self.pool_history.merge(std::mem::take(&mut p.operations));
            p
        });
        (pool, unfinished_shield, connecting_pool)
    }

    /// Asset locks in the history say what they were for: the pool's as a shield, the others
    /// with the identity they registered or top up.
    async fn say_what_locks_are(
        &self,
        id: WalletId,
        history: &mut [Activity],
        identities: &[IdentityChoice],
        lock_fors: &HashMap<String, identity::LockFor>,
        unfinished: Option<&UnfinishedIdentity>,
        unfinished_top_up: Option<&UnfinishedTopUp>,
    ) {
        let Some(wallet) = turn::timed("wallet_lookup_locks", self.chain.wallet(&id)).await else {
            return;
        };
        let pool_locks = turn::timed("pool_locks", self.chain.pool_locks(&wallet)).await;
        for act in history.iter_mut() {
            let locked = act.txid.as_ref().and_then(|t| pool_locks.get(t));
            if let (ActivityKind::TopUp { .. }, Some(locked)) = (&act.kind, locked) {
                act.kind = ActivityKind::Shielded {
                    from: Some("Core".into()),
                };
                // key-wallet's net for these locks counts the change but not the lock: what
                // left Core is the lock and its fee.
                act.amount = -(*locked as Duffs + act.fee.unwrap_or(0));
            }
        }
        // "Transfer to Platform" says whom — the identity it registered or tops up — and
        // whether Platform took it (BUG-57).
        let tracked = tracked_locks(&wallet).await;
        let top_ups = self.journal.top_ups();
        let chainlocked = self.chain.status().chainlock_height;
        let mut no_answer = Vec::new();
        let waiting = self
            .waiting_locks(&wallet, unfinished, unfinished_top_up)
            .await;
        for act in history.iter_mut() {
            let Some(txid) = act
                .txid
                .clone()
                .filter(|_| matches!(act.kind, ActivityKind::TopUp { .. }))
            else {
                continue;
            };
            let (who, made, mut taken) = identity::lock_line(
                &txid,
                lock_fors.get(&txid).copied(),
                tracked.get(&txid).copied(),
                self.consumed_locks.contains(&txid),
                waiting.contains(&txid),
                identities,
            );
            // What the wallet does not know, Platform may have proved (FEAT-0016).
            // A lock not added is asked too: it may have been used from another device since
            // (FEAT-0016 r1 Should 1).
            if lock_status::worth_asking(taken)
                && lock_status::core_proved(act.proof, act.block, chainlocked)
            {
                match self
                    .lock_status
                    .credits(&txid, made, taken == Credits::NotAdded)
                {
                    Some(answer) => taken = answer,
                    None => no_answer.push((txid.clone(), made)),
                }
            }
            say_the_lock(act, (who, made, taken), &top_ups, identities);
        }
        self.lock_status.want(no_answer);
    }

    /// The locks Identity offers to continue, and only while it offers that: the same rules as
    /// `Wallet::unfinished_*` (not while one is being made, only after the search for the
    /// identities).
    async fn waiting_locks(
        &self,
        wallet: &PlatformWallet,
        unfinished: Option<&UnfinishedIdentity>,
        unfinished_top_up: Option<&UnfinishedTopUp>,
    ) -> std::collections::HashSet<String> {
        let mut waiting = std::collections::HashSet::new();
        if unfinished_top_up.is_some() {
            for lock in
                turn::timed("unfinished_top_ups", self.chain.unfinished_top_ups(wallet)).await
            {
                if !lock.awaiting_chain_lock {
                    waiting.insert(lock.out_point.txid.to_string());
                }
            }
        }
        if unfinished.is_some_and(|u| u.locked.is_some())
            && let Some(lock) = turn::timed("unfinished_lock", self.chain.unfinished(wallet)).await
        {
            waiting.insert(lock.out_point.txid.to_string());
        }
        waiting
    }

    /// Payments to contacts say whom, and each contact counts them.
    async fn name_contact_lines(
        &self,
        id: WalletId,
        history: &mut [Activity],
        contacts: &mut [Contact],
    ) {
        let Some(wallet) = turn::timed("wallet_lookup_keys", self.chain.wallet(&id)).await else {
            return;
        };
        let mut keys = turn::timed(
            "contact_key_addresses",
            self.chain.contact_key_addresses(&wallet),
        )
        .await;
        // And the ones paid in earlier runs, kept beside the store.
        for c in contacts.iter() {
            for address in self.contact_paid.paid(&c.identity_id) {
                keys.insert(address, c.identity_id.clone());
            }
        }
        convert::name_contact_payments(history, &keys, contacts);
        convert::count_contact_payments(history, &keys, contacts);
    }

    /// Rescans owed for payments that lost their block, and the lines waiting on one.
    async fn settle_rescans(
        &mut self,
        id: WalletId,
        birth_height: u32,
        synced: u32,
        lost_block: Option<mined::Lost>,
        contact_accounts: &[String],
        history: &mut Vec<Activity>,
    ) {
        // A payment long out of a block may have lost its block before this file knew it
        // (BUG-64): the filters are scanned again from the wallet's birth to find it — once for
        // each such payment, also across restarts (review r1 should 1). Noted as done only when
        // the rescan has run to its end (review r2 should 1), and a lost block is noted too
        // (should 2): one cut short is asked again at the next start, one finished never again.
        if let Some(done) = OwedRescan::finished(&mut self.rescan_owed, &id, synced) {
            self.mempool.rescanned(&done.stale);
            self.mined.rescanned(&done.lost);
            self.contact_scans.done(&done.contacts);
            if let Some((asked, records)) = done.contacts_asked {
                crate::diag::record(crate::diag::Record::RescanDone {
                    accounts: done.contacts.len() as u32,
                    found: history.len().saturating_sub(records) as u32,
                    secs: asked.elapsed().as_secs(),
                });
            }
        }
        // Read once, before a rescan is asked: a rescan asked below is owed in this snapshot.
        let sync_done = self.chain.status().phase == sync::Phase::Synced;
        if let Some(lost) = lost_block {
            let from = lost.from.saturating_sub(1);
            let why = "lost_block";
            turn::timed(
                "rescan_lost",
                self.rescan_from(id, from, why, synced, Vec::new(), lost.txids),
            )
            .await;
        } else {
            let stale = self.mempool.not_rescanned(history);
            if !stale.is_empty() {
                turn::timed(
                    "rescan_stale",
                    self.rescan_from(id, birth_height, "stale", synced, stale, Vec::new()),
                )
                .await;
            }
        }
        // BUG-150: a contact's receiving account built after the filters went past its coins: once
        // each, from the birth, not again while one for it is under way.
        let under_way = OwedRescan::contacts_under_way(&self.rescan_owed, &id).to_vec();
        let owed: Vec<String> = self
            .contact_scans
            .owed(contact_accounts)
            .into_iter()
            .filter(|a| !under_way.contains(a))
            .collect();
        if !owed.is_empty() {
            turn::timed(
                "rescan_contacts",
                self.rescan_for_contacts(id, birth_height, synced, owed, history.len()),
            )
            .await;
        }
        // Stale since an earlier run and not rescanned yet: most likely in a block the store
        // lost, not a payment still coming or going (BUG-82). Left out only while its rescan
        // is ahead (not synced yet) or owed (review r1 must 1); shown once it has said.
        let owed = self
            .rescan_owed
            .as_ref()
            .filter(|o| o.wallet == id)
            .map_or(&[][..], |o| o.stale.as_slice());
        self.mempool.leave_out_checking(history, owed, sync_done);
    }

    /// The person's notes on their lines (FEAT-0003); what went wrong with the notes' file, if
    /// anything. A send's note waiting for its line goes there first, off the loop's thread:
    /// binding it writes the file (AC-T9, review r1 should 2). Rare: once per send out of the
    /// pool.
    async fn put_notes_on(&self, history: &mut [Activity]) -> Option<NotesError> {
        let to_bind = self
            .notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|notes| notes.lines_to_bind(history));
        if let Some(lines) = to_bind {
            let notes = self.notes.clone();
            let bound = tokio::task::spawn_blocking(move || {
                if let Some(notes) = notes.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                    notes.bind(&lines);
                }
            });
            let _ = turn::timed("notes_bind", bound).await;
        }
        let caught_up = coins_known(&self.chain.status(), true);
        let first = self.first_seen_keys(history, caught_up);
        self.rate_first_seen(first).await;
        match self
            .notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            Some(notes) => {
                for line in history.iter_mut().filter(|a| !a.key.is_empty()) {
                    line.note = notes.get(&line.key).map(str::to_owned);
                    line.rate = notes.rate(&line.key);
                }
                notes.trouble()
            }
            None => None,
        }
    }

    /// The fees the snapshot shows for a wallet of `coins` coins (without
    /// [`Fees::core_coins`]): the pool's and Platform's at the protocol version the SDK speaks,
    /// and what this data folder last really paid for each kind of write. One place, so what a
    /// form promises is what the core charges (BUG-54).
    pub(super) async fn fees_now(&self, coins: usize) -> Fees {
        let mut fees = fees(coins);
        if let Ok(sdk) = self.chain.sdk().await {
            pool::set_fees(&mut fees, sdk.version());
            platform_fees(&mut fees, sdk.version(), &self.journal);
        }
        fees
    }

    /// Reads the wallet's history from the store again when the wallet in memory has new
    /// records (`live_count` changed) or 10 s have passed: the store gets them a little after
    /// the wallet does. A read that fails keeps the last one.
    async fn refresh_stored_history(&mut self, id: WalletId, live_count: usize) {
        let fresh = self.history_read.is_some_and(|(count, at)| {
            count == live_count && at.elapsed() < Duration::from_secs(10)
        });
        if fresh {
            return;
        }
        self.history_read = Some((live_count, tokio::time::Instant::now()));
        let store = self.paths.chain.store.clone();
        let read = tokio::task::spawn_blocking(move || {
            (
                chain::stored_history(&store, &id),
                chain::consumed_locks(&store, &id),
            )
        })
        .await;
        if let Ok((records, consumed)) = read {
            if let Ok(records) = records {
                self.stored_history = records;
            }
            if let Ok(consumed) = consumed {
                self.consumed_locks = consumed;
            }
        }
    }

    /// Reads Platform back into the journal off the loop (BUG-58, R-33): first once the
    /// wallet's identities are known and proofs can be checked, then every [`HISTORY_EVERY`];
    /// at once again after a new identity lookup. One pass at a time. Its result is kept only
    /// for the wallet and the unlock it was started for. A failed pass (the network, or a
    /// journal that cannot be written) leaves the marks where they were and is tried again
    /// [`HISTORY_EVERY`] later.
    pub(super) fn read_platform_history(&mut self) {
        let Some(id) = self.unlocked.as_ref().map(|o| o.id) else {
            self.history_next = None;
            return;
        };
        if !self.identities_complete.load(Ordering::Relaxed) {
            // A lookup is running: the pass after it comes at once.
            self.history_next = None;
            return;
        }
        if !self.chain.platform_ready()
            || self.history_busy.load(Ordering::Relaxed)
            || self
                .history_next
                .is_some_and(|due| std::time::Instant::now() < due)
        {
            return;
        }
        self.history_next = Some(std::time::Instant::now() + HISTORY_EVERY);
        self.history_busy.store(true, Ordering::Relaxed);
        let busy = self.history_busy.clone();
        let run = self.discovery_run.load(Ordering::Relaxed);
        let current = self.discovery_run.clone();
        let chain = self.chain.clone();
        let journal = self.journal.clone();
        let marks_path = self.paths.platform_sync.clone();
        let names = self.names.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let refresh = self.refresher();
        tokio::spawn(async move {
            // Lowered on every way out, a panic in the read too.
            struct Done(Arc<AtomicBool>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Relaxed);
                }
            }
            let _done = Done(busy);
            let Some(wallet) = chain.wallet(&id).await else {
                return;
            };
            let marks = platform_history::Marks::load(&marks_path);
            let read = platform_history::read(&chain, &wallet, id, &marks, &names);
            match turn::timed("platform_history_read", read).await {
                // A lock (or a new lookup) since the start: this answer is for no one.
                Ok(_) if current.load(Ordering::Relaxed) != run => {}
                Ok((entries, next)) => {
                    // The entries first, then the marks: a crash between them reads again,
                    // and the keys drop the repeat. Not kept (the file's failure is in the
                    // journal already): the marks stay where they were, the next pass reads
                    // the same again.
                    if let Ok(changed) = journal.merge_network(entries) {
                        if changed {
                            refresh();
                        }
                        if next != marks {
                            let _ = next.save(&marks_path);
                        }
                    }
                }
                Err(e) => crate::diag::platform_failed("platform_history", &e),
            }
        });
    }
}

impl Real {
    /// Asks Platform, with a proof, about the locks the last snapshot could not say anything
    /// of (FEAT-0016): in the background, one request at a time, once a run. Only reads.
    pub(super) fn read_lock_status(&mut self) {
        if self.unlocked.is_none()
            || !self.chain.platform_ready()
            || self.lock_status_busy.load(Ordering::Relaxed)
        {
            return;
        }
        let batch = self.lock_status.next_batch(std::time::Instant::now());
        if batch.is_empty() {
            return;
        }
        self.lock_status_busy.store(true, Ordering::Relaxed);
        let busy = self.lock_status_busy.clone();
        let chain = self.chain.clone();
        let status = self.lock_status.clone();
        let refresh = self.refresher();
        tokio::spawn(async move {
            // Lowered on every way out, a panic in the read too.
            struct Done(Arc<AtomicBool>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Relaxed);
                }
            }
            let _done = Done(busy);
            let read = lock_status::read(&chain, &batch);
            match turn::timed("lock_status_read", read).await {
                Ok(answers) => {
                    for (lock, told) in answers {
                        match told {
                            Ok(told) => {
                                status.answered(&lock.0, told);
                            }
                            Err(e) => {
                                crate::diag::platform_failed("lock_status", &e);
                                status.failed(&[lock], std::time::Instant::now());
                            }
                        }
                    }
                    refresh();
                }
                Err(e) => {
                    crate::diag::platform_failed("lock_status", &e);
                    status.failed(&batch, std::time::Instant::now());
                }
            }
        });
    }
}

/// Keeps the identities' credit balances as Platform has them, while `live` holds:
/// at once, then after each write (`stale`) or every [`BALANCES_EVERY`]. The shown identity is
/// read each time, all of them at first and every [`BALANCES_ALL_EVERY`] times. `checked` is
/// raised once a round has read them all (BUG-115).
pub(super) async fn keep_balances_fresh(
    chain: Arc<Chain>,
    id: WalletId,
    live: impl Fn() -> bool,
    stale: Arc<tokio::sync::Notify>,
    checked: Arc<AtomicBool>,
    refresh: impl Fn(),
) {
    let mut round = 0u32;
    while live() {
        let all = round.is_multiple_of(BALANCES_ALL_EVERY);
        round = round.wrapping_add(1);
        if let Some(wallet) = chain.wallet(&id).await {
            let result =
                turn::timed("refresh_balances", chain.refresh_balances(&wallet, all)).await;
            if round_reads_all(all, &result) && live() {
                checked.store(true, Ordering::Relaxed);
            }
            if result.is_ok() {
                refresh();
            }
        }
        tokio::select! {
            _ = stale.notified() => {}
            _ = tokio::time::sleep(BALANCES_EVERY) => {}
        }
    }
}

/// Whether the credits are known (BUG-91): an identity is known here, or the search for them
/// ended and found none. While it runs or after it failed, a 0 is not an amount.
fn credits_known(has_identities: bool, search: &IdentitySearch) -> bool {
    has_identities || *search == IdentitySearch::Done
}

/// Whether the credits shown were checked in this run (BUG-115): every identity's read from
/// Platform (`read_all`) or there is none (`has_identities` false), and the search for them over,
/// so no other one is coming.
fn credits_checked(read_all: bool, has_identities: bool, search: &IdentitySearch) -> bool {
    (read_all || !has_identities) && *search == IdentitySearch::Done
}

/// Whether a round of [`keep_balances_fresh`] read every identity's credits: one of all of them
/// (`all`), none failed. `result`: how many it read and how many it asked.
fn round_reads_all(all: bool, result: &Result<(usize, usize), String>) -> bool {
    all && matches!(result, Ok((read, asked)) if read == asked)
}

/// Whether the Platform address's balance is known (BUG-91): Platform answered this run, or the
/// store holds credits from an earlier read. A stored 0 is what an unread address has as well.
fn platform_address_known(answered_this_run: bool, stored_credits: u64) -> bool {
    answered_this_run || stored_credits > 0
}

#[cfg(test)]
mod platform_known_tests {
    use super::{credits_checked, credits_known, platform_address_known, round_reads_all};
    use evolite_api::IdentitySearch::{Done, Failed, Searching};

    /// BUG-115: the credits kept in the store are not checked until Platform was asked for every
    /// identity this run and the search ended; a failed search leaves them unchecked.
    #[test]
    fn credits_are_checked_once_every_identity_was_read_and_the_search_ended() {
        assert!(credits_checked(true, true, &Done));
        assert!(!credits_checked(true, true, &Searching));
        assert!(!credits_checked(true, true, &Failed("no".into())));
        assert!(!credits_checked(false, true, &Done), "kept, not read");
        assert!(
            credits_checked(false, false, &Done),
            "none found: nothing to check"
        );
        assert!(!credits_checked(false, false, &Searching));
    }

    /// BUG-115: only a round of all of them with none failed reads them all.
    #[test]
    fn a_round_reads_all_only_when_it_asked_all_and_none_failed() {
        assert!(round_reads_all(true, &Ok((3, 3))));
        assert!(!round_reads_all(true, &Ok((2, 3))), "one failed");
        assert!(!round_reads_all(false, &Ok((1, 1))), "the shown one only");
        assert!(!round_reads_all(true, &Err("no".into())));
    }

    #[test]
    fn credits_are_known_once_an_identity_is_or_the_search_ended() {
        assert!(!credits_known(false, &Searching));
        assert!(!credits_known(false, &Failed("no".into())));
        assert!(credits_known(false, &Done));
        assert!(credits_known(true, &Searching));
        assert!(credits_known(true, &Failed("no".into())));
        assert!(credits_known(true, &Done));
    }

    #[test]
    fn the_address_is_known_once_platform_answered_or_the_store_has_credits() {
        assert!(!platform_address_known(false, 0));
        assert!(platform_address_known(false, 5));
        assert!(platform_address_known(true, 0));
    }
}

/// The two sides of an incoming request, from its journal key `cri:<from>:<ours>`: the sender
/// and, when the key has it, the identity of ours it came to.
fn incoming_sides(key: &str) -> Option<(&str, Option<&str>)> {
    let sides = key.strip_prefix("cri:")?;
    Some(match sides.split_once(':') {
        Some((from, ours)) => (from, Some(ours)),
        None => (sides, None),
    })
}

/// Who is a contact of each of the wallet's identities: a request came to the one it says.
fn answered_requests(
    mine: &[&platform_wallet::ManagedIdentity],
) -> HashMap<String, std::collections::HashSet<String>> {
    answered_by(mine.iter().map(|m| {
        (
            m.identity.id().to_string(Encoding::Base58),
            m.dashpay().established_contacts(),
        )
    }))
}

/// [`answered_requests`] from each identity's id and its established contacts (a request one side
/// has only sent is not among them).
fn answered_by<'a, C: 'a>(
    mine: impl Iterator<Item = (String, &'a std::collections::BTreeMap<Identifier, C>)>,
) -> HashMap<String, std::collections::HashSet<String>> {
    mine.map(|(ours, contacts)| (ours, contacts.keys().map(dashpay::base58).collect()))
        .collect()
}

/// What the line of an incoming request says now: hidden on this computer, accepted once the
/// sender is a contact of the identity it came to, else as the journal has it. The running
/// wallet knows the contact at once, before the next read of Platform (BUG-88). `answered` says
/// whether a sender is a contact of the identity of ours the request came to (BUG-110: any of the
/// wallet's, not only the one shown).
fn settle_incoming<'a>(
    state: evolite_api::RequestState,
    key: Option<&'a str>,
    hidden: impl Fn(&'a str) -> bool,
    answered: impl Fn(&'a str, &'a str) -> bool,
) -> evolite_api::RequestState {
    use evolite_api::RequestState::{Accepted, Hidden, Waiting};
    let Some((from, ours)) = key.and_then(incoming_sides) else {
        return state;
    };
    if state != Waiting {
        return state;
    }
    if hidden(from) {
        Hidden
    } else if ours.is_some_and(|ours| answered(ours, from)) {
        Accepted
    } else {
        state
    }
}

#[cfg(test)]
mod incoming_tests {
    use super::{Encoding, Identifier, answered_by, incoming_sides, settle_incoming};
    use evolite_api::RequestState::{self, Accepted, Hidden, Waiting};

    /// Settled with the contacts of identity `s`, the one shown.
    fn settled(state: RequestState, key: &str, contacts: &[&str]) -> RequestState {
        let answered: std::collections::HashSet<&str> = contacts.iter().copied().collect();
        settle_incoming(
            state,
            Some(key),
            |_| false,
            |ours, f| ours == "s" && answered.contains(f),
        )
    }

    #[test]
    fn a_key_names_the_sender_and_the_identity_it_came_to() {
        assert_eq!(incoming_sides("cri:x:me"), Some(("x", Some("me"))));
        assert_eq!(incoming_sides("cri:x"), Some(("x", None)));
        assert_eq!(incoming_sides("cr:me:x"), None);
    }

    #[test]
    fn a_request_to_the_identity_shown_says_accepted_once_it_is_a_contact() {
        assert_eq!(settled(Waiting, "cri:x:s", &["x"]), Accepted);
    }

    /// X is a contact of S; its request to T has not been answered by T (r1, must 1).
    #[test]
    fn a_request_to_another_identity_is_not_accepted_by_a_contact_of_the_one_shown() {
        assert_eq!(settled(Waiting, "cri:x:t", &["x"]), Waiting);
    }

    /// BUG-110: X became a contact of T while S was shown: T's request says accepted at once.
    #[test]
    fn a_request_to_another_identity_says_accepted_once_that_identity_has_the_contact() {
        let answered: std::collections::HashSet<&str> = ["x"].into_iter().collect();
        let state = settle_incoming(
            Waiting,
            Some("cri:x:t"),
            |_| false,
            |ours, f| ours == "t" && answered.contains(f),
        );
        assert_eq!(state, Accepted);
    }

    /// One way only: not among the established contacts, so not answered.
    #[test]
    fn a_request_one_side_has_only_sent_is_not_accepted() {
        assert_eq!(settled(Waiting, "cri:x:s", &[]), Waiting);
    }

    /// BUG-110 r1 should 2: the map a line is settled by holds each identity's own contacts.
    #[test]
    fn each_identity_answers_for_its_own_contacts() {
        let (s, t, x, y) = (
            Identifier::from([1; 32]),
            Identifier::from([2; 32]),
            Identifier::from([3; 32]),
            Identifier::from([4; 32]),
        );
        let b58 = |id: &Identifier| id.to_string(Encoding::Base58);
        let of_s = std::collections::BTreeMap::from([(x, ())]);
        let of_t = std::collections::BTreeMap::from([(y, ()), (x, ())]);
        let answered = answered_by([(b58(&s), &of_s), (b58(&t), &of_t)].into_iter());
        let holds = |ours: &Identifier, f: &Identifier| answered[&b58(ours)].contains(&b58(f));
        assert!(holds(&s, &x) && holds(&t, &x) && holds(&t, &y));
        assert!(!holds(&s, &y), "a contact of t is no contact of s");
    }

    #[test]
    fn hidden_goes_by_the_sender_whichever_identity_is_shown() {
        for key in ["cri:x:s", "cri:x:t", "cri:x"] {
            assert_eq!(
                settle_incoming(Waiting, Some(key), |_| true, |_, _| false),
                Hidden,
                "{key}"
            );
        }
    }

    #[test]
    fn only_a_waiting_line_changes() {
        assert_eq!(
            settle_incoming(Accepted, Some("cri:x:s"), |_| true, |_, _| false),
            Accepted
        );
        assert_eq!(
            settle_incoming(Waiting, None, |_| true, |_, _| true),
            Waiting
        );
        assert_eq!(
            settle_incoming(Waiting, Some("cri:x"), |_| false, |_, _| true),
            Waiting,
            "a key without our side: nobody can tell"
        );
    }
}
