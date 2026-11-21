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

/// A day of blocks before the contacts' own heights (BUG-221 r1 Should): `coreHeightCreatedAt`
/// is written by the sender's client, and our outgoing request may come from another wallet on
/// the same phrase that wrote one too high. It costs seconds.
const RESCAN_MARGIN: u32 = 576;

/// SYNC-7: where the contacts' rescan starts: [`RESCAN_MARGIN`] before the lowest height an
/// `owed` account's contact begins at, never below the wallet's `birth`; the birth when one of
/// them has no such height.
fn rescan_start(accounts: &[(String, Option<u32>)], owed: &[String], birth: u32) -> u32 {
    owed.iter()
        .map(|key| {
            accounts
                .iter()
                .find(|(k, _)| k == key)
                .and_then(|(_, begins)| *begins)
        })
        .try_fold(u32::MAX, |low, begins| begins.map(|b| low.min(b)))
        .map_or(birth, |low| low.saturating_sub(RESCAN_MARGIN).max(birth))
}

#[cfg(test)]
mod rescan_start_tests {
    use super::rescan_start;

    /// SYNC-7: the rescan for owed contacts starts a day of blocks before the earliest of them
    /// begins (r1 Should: a height written too high by the sender's client), never below the
    /// birth; one without a height (not established) falls back to the birth; an
    /// account not owed does not lower it.
    #[test]
    fn the_contacts_rescan_starts_where_they_begin() {
        let accounts = vec![
            ("a".to_owned(), Some(1_590_000)),
            ("b".to_owned(), Some(1_591_500)),
            ("old".to_owned(), Some(900_000)),
            ("none".to_owned(), None),
        ];
        let owed = |keys: &[&str]| keys.iter().map(|k| (*k).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            rescan_start(&accounts, &owed(&["a", "b"]), 975_000),
            1_590_000 - 576
        );
        assert_eq!(
            rescan_start(&accounts, &owed(&["b"]), 975_000),
            1_591_500 - 576
        );
        // BUG-221 r1 Should: a day before, but still never below the birth.
        assert_eq!(rescan_start(&accounts, &owed(&["a"]), 1_589_800), 1_589_800);
        assert_eq!(
            rescan_start(&accounts, &owed(&["old"]), 975_000),
            975_000,
            "not below the birth"
        );
        assert_eq!(
            rescan_start(&accounts, &owed(&["a", "none"]), 975_000),
            975_000
        );
        assert_eq!(rescan_start(&accounts, &owed(&["gone"]), 975_000), 975_000);
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

/// The identity a task acts as (TECH-signing-identity), in base58 for its History line and as the
/// id every check, credits read and signature of the task takes: the one the screen showed when
/// its command came (`pinned`, [`Real::shown_now`] read before the task, BUG-206), not the
/// one it names by the time the task runs.
pub(in crate::real) async fn pinned_acting(
    wallet: &PlatformWallet,
    pinned: Option<Identifier>,
) -> Option<(String, Identifier)> {
    let id = pinned_in(pinned, wallet_identity_ids(wallet).await.into_iter())?;
    Some((id.to_string(Encoding::Base58), id))
}

/// The pinned identity if the wallet (`mine`, base58 with slots) still has it. Never another one
/// in its place (BUG-206 r1): an identity shown and confirmed that is gone is a refusal, not the
/// lowest one paying instead.
fn pinned_in(
    pinned: Option<Identifier>,
    mut mine: impl Iterator<Item = (String, u32)>,
) -> Option<Identifier> {
    let pinned = pinned?;
    let shown = pinned.to_string(Encoding::Base58);
    mine.any(|(id, _)| id == shown).then_some(pinned)
}

impl Real {
    /// What a command acts as, pinned when it comes (BUG-206): the identity the screen showed
    /// for wallet `id` in the last snapshot; `None` before one showed any.
    pub(in crate::real) fn shown_now(&self, id: WalletId) -> Option<Identifier> {
        self.shown_now.filter(|(w, _)| *w == id).map(|(_, i)| i)
    }
}

/// Every identity of this wallet, in base58, with its slot (FEAT-0021 R-7: two of them never
/// contest one name).
pub(in crate::real) async fn wallet_identity_ids(wallet: &PlatformWallet) -> Vec<(String, u32)> {
    let state = wallet.state().await;
    let id = wallet.wallet_id();
    state
        .identity_manager
        .managed_identities()
        .filter(|m| m.wallet_id == Some(id))
        .map(|m| {
            (
                m.identity.id().to_string(Encoding::Base58),
                m.identity_index.unwrap_or_default(),
            )
        })
        .collect()
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
            failing: pool.failing.map(str::to_owned),
        },
        None => Pool {
            notes: 0,
            checked_at: None,
            address: None,
            unfinished_shield,
            connecting,
            operations: Vec::new(),
            failing: None,
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
    /// The values of the coins an asset lock may take: final and signable (SPV item 3).
    lock_values: Vec<u64>,
    /// What of the balance no payment can sign for yet (BUG-142).
    unsignable: Duffs,
    /// The contacts' receiving accounts, by [`contact_scans::ContactScans::key`] (BUG-150).
    /// Each DashPay receiving account, with the height its contact begins at (SYNC-7).
    contact_accounts: Vec<(String, Option<u32>)>,
    history: Vec<Activity>,
    shown: Option<ShownIdentity>,
    platform: Option<(String, u64)>,
    unfinished: Option<UnfinishedIdentity>,
    unfinished_top_up: Option<UnfinishedTopUp>,
    /// Duffs locked for a funding of the Platform address not taken yet (FEAT-0015 stage 2),
    /// and whether it holds a new one back (BUG-160).
    /// With it: the address «Continue» credits and whether its lock has an InstantLock (BUG-171).
    unfinished_address_fund: Option<(u64, bool, String, bool)>,
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
        // SYNC-0: when Platform's proved reads may go, once per unlock.
        if self.unlocked.is_some() && self.chain.platform_ready() {
            crate::diag::platform_ready("network");
        }
        let status = self.chain.status();
        // SPV item 2a: the tip header's time, for «no new blocks».
        let tip_time = turn::timed("tip_block_time", self.chain.tip_block_time()).await;
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
                platform: self.works.now(),
                core_node_next: {
                    let saved =
                        turn::during("core_node_file", || node::load(&self.paths.chain.core_node));
                    // Empty: back to DNS seeds at the next start.
                    (saved != status.core_node).then(|| saved.unwrap_or_default())
                },
                ..self.shown_sync(&status, tip_time)
            },
            vault,
            wallet,
            demo: false,
            elsewhere: false,
            fiat: self.fiat.lock().map(|f| f.clone()).unwrap_or_default(),
            update: self.update.lock().map(|u| u.clone()).unwrap_or_default(),
            advanced_features: self.advanced_features,
            advanced_allowed: self.advanced_allowed(),
            wallets: self.wallet_entries(),
            adding_wallet: self.adding_from.is_some(),
        }
    }

    /// The sync as the screens show it. A rescan for the contacts' receiving accounts (BUG-150) takes
    /// the filters back to the birth in the background: the screens go on saying «Done», not
    /// «syncing» from the start again (the architect). A payment waits for it all the same.
    fn shown_sync(&self, status: &sync::SyncStatus, tip_time: Option<u32>) -> SyncInfo {
        let mut shown = sync_info(status, self.unlocked.is_some());
        let rescanning = self
            .unlocked
            .as_ref()
            .is_some_and(|o| !OwedRescan::contacts_under_way(&self.rescan_owed, &o.id).is_empty());
        if rescanning && status.phase != sync::Phase::Error {
            shown.stage = SyncStage::Done;
            shown.progress = None;
            // BUG-221: «Done», and the balance said to be possibly incomplete until it ends.
            shown.contacts_rescan = true;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as evolite_api::Timestamp);
        shown.tip_at = self.tip_seen_at(shown.tip_height, now);
        // SPV item 2a: no new block for a while — by both clocks — only once synced.
        let clock = self.tip_clock(shown.tip_height, shown.stage == SyncStage::Done);
        shown.stale_minutes = match clock.synced_for {
            Some(synced_for) => stale_minutes(
                TipGrowth {
                    since_grew: clock.since_grew,
                    grew_this_run: clock.grew_this_run,
                    synced_for,
                },
                tip_time.map(i64::from),
                now,
            ),
            None => None,
        };
        // The journal says it once, on entering the state (AC-T2а.3).
        let was = self
            .stale_noted
            .swap(shown.stale_minutes.is_some(), Ordering::Relaxed);
        if let (false, Some(minutes)) = (was, shown.stale_minutes) {
            crate::diag::record(crate::diag::Record::SyncStale { minutes });
        }
        shown
    }

    /// The tip on this run's steady clock (SPV item 2a): how long it has not grown, whether it
    /// grew at all since the start, and how long the wallet has been synced (`None` while it is
    /// not). The first tip seen (the store's, at the start) is no growth. A wall clock moved
    /// meanwhile does not count.
    fn tip_clock(&self, tip: u32, synced: bool) -> TipClockNow {
        let mut clock = self.tip_grew.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        if clock.tip == 0 {
            clock.tip = tip;
            clock.grew_at = now;
        } else if tip > clock.tip {
            clock.tip = tip;
            clock.grew_at = now;
            clock.grew_this_run = true;
        }
        if !synced {
            clock.synced_at = None;
        } else if clock.synced_at.is_none() {
            clock.synced_at = Some(now);
        }
        TipClockNow {
            since_grew: now.duration_since(clock.grew_at).as_secs(),
            grew_this_run: clock.grew_this_run,
            synced_for: clock.synced_at.map(|at| now.duration_since(at).as_secs()),
        }
    }

    /// When the tip last grew (BUG-155 AC-A9): a higher `tip` than seen before is new at `now`.
    fn tip_seen_at(&self, tip: u32, now: evolite_api::Timestamp) -> Option<evolite_api::Timestamp> {
        let mut seen = self.tip_seen.lock().unwrap_or_else(|e| e.into_inner());
        tip_seen(&mut seen, tip, now)
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
    /// phrase is one wallet here, not two folders syncing the same keys. FEAT-0041 R-12: by the
    /// seed — the same phrase with another BIP-39 passphrase is another wallet, and is made.
    pub(super) fn same_wallet(&self, secret: &vault::Secret) -> Option<String> {
        let seed = evolite_chain::seed_from(&secret.phrase, &secret.passphrase).ok()?;
        let id = chain::wallet_id_of(&seed).ok()?;
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
        // BUG-162: how far this wallet itself is synced — its coins are known by that, not by
        // the client's filters, which may have reached the tip before the wallet joined.
        if let Some(wallet) = turn::timed("wallet_synced", self.chain.wallet(&id)).await {
            let height = wallet.state().await.core_wallet.synced_height();
            self.chain.note_wallet_synced(Some(height));
        }
        // Read before the coins are counted, and again after ([`count_checked`]).
        let status_before = self.chain.status();
        let WalletRead {
            balance,
            coins,
            addresses,
            coin_values,
            lock_values,
            unsignable,
            contact_accounts,
            mut history,
            shown,
            platform,
            unfinished,
            unfinished_top_up,
            unfinished_address_fund,
            lost_block,
            synced,
            identities,
            lock_fors,
            all_credits,
            own_core,
            mixed,
        } = self.read_wallet(id).await;
        // The height its coins were read at: the «after» of [`count_checked`] sees it.
        self.chain.note_wallet_synced(Some(synced));
        let (platform_address, platform_credits) = platform.unwrap_or_default();
        let (pool, unfinished_shield, connecting_pool) = self.read_pool(id).await;
        let mut fees = turn::timed("fees_now", self.fees_now(coins)).await;
        fees.core_coins = convert::core_coins(coin_values);
        fees.lock_coins = convert::core_coins(lock_values);
        fees.core_unsignable = unsignable;
        // A funding of the address cut short: finished to the address it was signed for (or,
        // after a restore, the one shown), with what Platform keeps: one estimate, whatever the
        // proof (BUG-171 AC-T5).
        let unfinished_address_top_up =
            unfinished_address_fund.map(|(locked, holds_back, address, _instant)| {
                evolite_api::UnfinishedAddressTopUp {
                    locked: locked as Duffs,
                    address,
                    on_top: fees.address_fund_on_top(),
                    holds_back,
                }
            });
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
        // Unnamed, and named only from this computer's file: read from the network (SYNC-4).
        let unnamed = {
            let cached = self.names_cached.lock().unwrap_or_else(|e| e.into_inner());
            contacts
                .iter()
                .filter(|c| c.name.is_none() || cached.contains(&c.identity_id))
                .map(|c| c.identity_id.clone())
                .collect()
        };
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
        let me = shown.as_ref().map(|(identity, _, _)| identity.id.clone());
        // FEAT-0020-resend: what the shown identity owes, and its batches (R-5, R-6, R-8, R-9).
        let resend = me.as_deref().and_then(|me| {
            super::ops::resend::shown(&self.resend_debt.load(), me, self.resend_run.of(me))
        });
        self.name_contact_lines(id, me.as_deref(), &mut history, &mut contacts)
            .await;
        convert::settle(&mut history, self.chain.status().chainlock_height);
        // BUG-173: own InstantLocks kept across a restart; «Confirmed · checking» from them.
        self.keep_instant_locks(&mut history);
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
        // SYNC-0: History's Platform lines shown from this computer's journal.
        if self.journal.count() > 0 {
            crate::diag::fresh("history", "store");
        }
        history.sort_by_key(|a| std::cmp::Reverse(a.at));
        // Only a checked balance is shown: while the filters are not at the tip (a start, a
        // rewound pass, a rescan) the last checked one with its time, or none (BUG-77).
        let (core, core_as_of) = self.core_balance.shown(
            balance.unwrap_or(0),
            count_checked(balance.is_some(), &status_before, &self.chain.status()),
            now(),
        );
        let mut shown_wallet = Wallet {
            bip39_passphrase: self.unlocked.as_ref().is_some_and(|u| u.passphrase),
            password_each_send: self.password_each_send,
            large_spend: self.large_spend,
            series: self.series_states(),
            identity_links: self.identity_links.load(Ordering::Relaxed),
            notes_error,
            set_aside: self.set_aside_files(),
            tokens: self.shown_tokens(
                shown.as_ref().map(|(identity, _, _)| identity.id.as_str()),
                identities.iter().map(|c| c.id.clone()).collect(),
            ),
            platform_inputs: self.shown_address_inputs.iter().map(|(_, c)| *c).collect(),
            platform_input_addresses: self
                .shown_address_inputs
                .iter()
                .map(|(a, _)| a.clone())
                .collect(),
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
            resend,
            unfinished_identity: unfinished,
            unfinished_top_up,
            unfinished_address_top_up,
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
        // Made here and cut short, or found on chain after a restore, and not proved taken by
        // Platform (FEAT-0015 AC-T6, BUG-160).
        let unfinished_address_fund =
            turn::timed("unfinished_address_fund", self.waiting_fund(&wallet))
                .await
                .map(|l| {
                    let failed = self
                        .fund_resume_failed
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    let shown = platform
                        .as_ref()
                        .map(|(a, _)| a.as_str())
                        .unwrap_or_default();
                    let address = super::ops::address_fund::resume_address(
                        self.fund_intents.address(&l.txid),
                        shown,
                    );
                    (l.amount, !failed.contains(&l.txid), address, l.instant)
                });
        let unfinished_top_up =
            turn::timed("unfinished_top_up", self.unfinished_top_up(&wallet)).await;
        let live_count = {
            let state = turn::timed("state_count", wallet.state()).await;
            state.core_wallet.transaction_history().len()
        };
        // BUG-194: the pools as they are now, written to the store (platform-wallet does not):
        // first after the load, then on each new record.
        if self.pools_saved != Some((id, live_count)) {
            turn::timed("save_pools", self.chain.save_pools(&wallet)).await;
            self.pools_saved = Some((id, live_count));
        }
        turn::timed(
            "refresh_stored_history",
            self.refresh_stored_history(id, live_count),
        )
        .await;
        // BUG-232: a payment to an address of ours past the pools' gap, before History is read.
        turn::timed(
            "adopt_past_the_gap",
            self.adopt_past_the_gap(id, &wallet, live_count),
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
        let mut records = self.activity_lines(&all, &coinjoin);
        // BUG-194: a payment from one of this wallet's identities to another, one line.
        let own_identities: HashMap<String, String> = state
            .identity_manager
            .managed_identities()
            .filter(|m| m.wallet_id == Some(id))
            .map(|m| {
                let id = m.identity.id().to_string(Encoding::Base58);
                let named = self.identity_of(m).name().map(|n| format!("@{n}"));
                (id.clone(), named.unwrap_or(id))
            })
            .collect();
        convert::name_identity_transfers(
            &mut records,
            &convert::own_contact_receipts(&state.core_wallet),
            &own_identities,
        );
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
                name: self.identity_of(m).name().map(str::to_owned),
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
        // BUG-206 r1: what the next command acts as is what this snapshot shows.
        self.shown_now = first.map(|m| (id, m.identity.id()));
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
            lock_values: convert::payable_coins(&state.core_wallet, |txid| {
                self.chain.instant_lock_verified(txid)
            })
            .iter()
            .map(|u| u.value())
            .collect(),
            unsignable: convert::unsignable(&state.core_wallet),
            contact_accounts: state
                .core_wallet
                .accounts
                .dashpay_receival_accounts
                .keys()
                .map(|k| {
                    let key = contact_scans::ContactScans::key(
                        &k.user_identity_id,
                        &k.friend_identity_id,
                        k.index,
                    );
                    // SYNC-7: the contact begins at the earlier of its two requests'
                    // `coreHeightCreatedAt` (platform-wallet's own checkpoint): nothing of it
                    // is in a block before that.
                    let begins = state
                        .identity_manager
                        .managed_identity(&Identifier::from(k.user_identity_id))
                        .and_then(|m| {
                            m.dashpay()
                                .established_contacts()
                                .get(&Identifier::from(k.friend_identity_id))
                        })
                        .map(|e| {
                            e.outgoing_request
                                .core_height_created_at
                                .min(e.incoming_request.core_height_created_at)
                        });
                    (key, begins)
                })
                .collect(),
            history,
            shown,
            platform,
            unfinished,
            unfinished_top_up,
            unfinished_address_fund,
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

    /// One record per transaction: the store's row and this run's records of every account it
    /// touches, merged by [`convert::one_per_transaction`] (the stored row first) — each input and
    /// output once over the accounts (BUG-85, BUG-237), so an asset lock, which touches two
    /// accounts, is one line, and a short stored row does not hide the live accounts' views.
    fn merged_records(
        &self,
        info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    ) -> Vec<TransactionRecord> {
        // BUG-237: the stored row and this run's slices of a transaction as one — the stored row
        // first, then the live accounts' views — so a short stored row does not hide the full
        // live one.
        let slices = info.transaction_history();
        let mut all = convert::one_per_transaction(self.stored_history.iter().chain(slices));
        // A spend recorded before the coin it spends (a restore's rescan) gets its
        // input back from our own outputs.
        // BUG-194: ours is the store's pools and the live wallet's — the store's end at the
        // first gap window, the live ones after a restart may end before an address the store
        // has. One rule for both steps (BUG-232, QA batch 31): the outputs a stored slice
        // lists, then those it did not list once its inputs are ours.
        let live = convert::live_own(info);
        let ours = |address: &key_wallet::dashcore::Address| {
            let script = address.script_pubkey();
            live.get(&script)
                .or_else(|| self.stored_own.get(script.as_bytes()))
                .copied()
        };
        for record in &mut all {
            convert::own_outputs(record, ours);
        }
        convert::attribute_late_inputs(&mut all, ours);
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

    /// What the interface shows of `m`, its names with the wallet's requests laid over them
    /// (FEAT-0021, AC-T3): a contested name platform-wallet lists once sent is still in its vote.
    fn identity_of(&self, m: &ManagedIdentity) -> Identity {
        let mut api = identity::to_api(m);
        let requests = self.name_requests.load();
        name_requests::apply(&mut api.names, &api.id, &requests, now());
        api
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
        let mut api = self.identity_of(m);
        // Whose each key is, while the wallet is open (FEAT-0020).
        self.fill_key_owners(&mut api, m);
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
        let entries = self.journal.entries();
        // BUG-191: a contested name won says so by its result line alone.
        let shown = entries
            .iter()
            .filter(|e| !journal::won_by_contest(e, &entries));
        history.extend(shown.map(|entry| {
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
        // FEAT-0015 stage 2 (AC-T7): a lock that funds the wallet's Platform address is «Transfer
        // from Core to the Platform address», with what left Core, the lock and its fee.
        let fund_locks =
            turn::timed("address_fund_locks", self.chain.address_fund_locks(&wallet)).await;
        for act in history.iter_mut() {
            if !matches!(act.kind, ActivityKind::TopUp { .. }) {
                continue;
            }
            let Some(txid) = act.txid.as_ref() else {
                continue;
            };
            if let Some(locked) = fund_locks.get(txid) {
                act.kind = ActivityKind::AddressFunded;
                act.amount = -(*locked as Duffs + act.fee.unwrap_or(0));
            } else if self.fund_intents.is_funding(txid) {
                // BUG-173: before platform-wallet's store is read (a start) our own record names
                // it, so the line does not change its name when the store comes; what left Core
                // is the transaction's own.
                act.kind = ActivityKind::AddressFunded;
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
        // FEAT-0015 AC-T6, BUG-160: every funding of the address not seen taken is asked about
        // too — made here or found on chain after a restore. A proved «spent» closes it (the
        // window may have closed after Platform took it); a recovered one waits for «not seen».
        let open = turn::timed("open_fund_locks", self.chain.open_fund_locks(&wallet)).await;
        // BUG-171, BUG-173: a record whose lock is no longer open is marked taken (it still names
        // the line); old and surplus ones go.
        self.fund_intents.settle(
            &open.iter().map(|l| l.txid.clone()).collect::<Vec<_>>(),
            evolite_chain::fund_intents::unix_now(),
        );
        self.lock_status.want_also(
            open.into_iter()
                .filter(|l| self.lock_status.answer(&l.txid).is_none())
                .map(|l| (l.txid, false))
                .collect(),
        );
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

    /// Payments to contacts say whom, and each of `me`'s contacts counts `me`'s (BUG-196).
    async fn name_contact_lines(
        &self,
        id: WalletId,
        me: Option<&str>,
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
        // BUG-212: the wallet's own payments to addresses of contacts' keys are paid ones — kept
        // in memory and written where the file lacks them (a file set aside or started over).
        let seen: Vec<(String, String)> = history
            .iter()
            .filter(|a| a.amount < 0)
            .filter_map(|a| {
                let address = a.address.as_ref()?;
                Some((keys.get(address)?.1.clone(), address.clone()))
            })
            .collect();
        if !seen.is_empty() {
            self.contact_paid.saw(&seen);
        }
        // And the ones paid in earlier runs, kept beside the store by the contact only: they
        // name the line; they count for no one unless platform-wallet's account has them, with
        // its pair (BUG-196).
        for c in contacts.iter() {
            for address in self.contact_paid.paid(&c.identity_id) {
                keys.entry(address)
                    .or_insert_with(|| (String::new(), c.identity_id.clone()));
            }
        }
        convert::name_contact_payments(history, &keys, contacts);
        convert::count_contact_payments(history, &keys, me, contacts);
    }

    /// How many coins the wallet's DashPay receiving accounts hold (BUG-221: what a contacts'
    /// rescan found).
    async fn dashpay_coins(&self, id: &WalletId) -> usize {
        match self.chain.wallet(id).await {
            Some(wallet) => convert::coin_kinds(&wallet.state().await.core_wallet)[2].0 as usize,
            None => 0,
        }
    }

    /// BUG-232 (qa-f29, tx1): on a fresh copy of an older data folder, key-wallet read a payment
    /// to our own change before its pool had made that address (a pool grows only from what it
    /// has seen used), so History showed the change as sent, until a restart loaded the longer
    /// pool. Once the filters are synced, and again when the wallet has new records, the outputs
    /// not known as ours are looked for in the BIP44 and BIP32 pools a little past their end;
    /// those found are taken in (History reads them as ours from this snapshot on), and the pools
    /// go to the store; so are addresses the pools have made already but not marked used (review
    /// r1 S1: a wallet restarted since, with the store's longer pool). A rescan from the first of
    /// their blocks is owed (`settle_rescans`): its blocks give the wallet their coins — key-wallet
    /// credits an output that became ours in a chain-locked record it reads again (vendor patch
    /// `evolite-finalized-outputs.patch`, review r1 M1) — and the payments onward that are new to
    /// it. The search runs on copies under the read lock; the write lock only puts a changed pool
    /// in place.
    async fn adopt_past_the_gap(
        &mut self,
        id: WalletId,
        wallet: &PlatformWallet,
        live_count: usize,
    ) {
        // During the pass the pools grow by themselves, and the records move every snapshot.
        if self.chain.status().phase != sync::Phase::Synced {
            return;
        }
        let key = (id, live_count, self.stored_history.len());
        if self.gap_looked == Some(key)
            || self
                .unlocked
                .as_ref()
                .and_then(|o| o.keys.as_ref())
                .is_none()
        {
            return;
        }
        self.gap_looked = Some(key);
        let Some(keys) = self.unlocked.as_ref().and_then(|o| o.keys.as_ref()) else {
            return;
        };
        let (finds, paid, uncredited) = {
            let state = wallet.state().await;
            let all = self.merged_records(&state.core_wallet);
            let paid = convert::paid_outside(&all);
            let addresses: Vec<_> = paid.iter().map(|(a, _)| a.clone()).collect();
            (
                convert::find_past_the_gap(&state.core_wallet, keys, &addresses),
                paid,
                convert::uncredited_outputs(&state.core_wallet, &all),
            )
        };
        // Review r1: outputs of ours already in the pools with no coin yet (taken in before a
        // close that came ahead of their rescan, or by a restart's longer pool): their rescan
        // is owed again — not while one is owed, and (review r2) not for ever: once, or twice
        // when the first gives no coin (`gap-rescans.json`).
        let uncredited: Vec<_> = uncredited
            .into_iter()
            .filter(|(o, _)| !self.gap_pending.contains(&(id, *o)))
            .collect();
        let keys: Vec<String> = uncredited.iter().map(|(o, _)| gap_key(o)).collect();
        let owed = self.gap_scans.owed(&keys);
        let owed: Vec<_> = uncredited
            .into_iter()
            .filter(|(o, _)| owed.contains(&gap_key(o)))
            .collect();
        self.gap_pending.extend(owed.iter().map(|(o, _)| (id, *o)));
        if let Some(from) = owed.iter().map(|(_, h)| *h).filter(|h| *h > 0).min() {
            super::note_past_the_gap(&mut self.past_the_gap_from, id, from.saturating_sub(1));
        }
        if finds.is_empty() {
            return;
        }
        let (adopted, (change_used, change_made)) = {
            let mut state = wallet.state_mut().await;
            let adopted = convert::take_in_past_the_gap(&mut state.core_wallet, finds);
            (adopted, convert::change_pool_ends(&state.core_wallet))
        };
        if adopted.is_empty() {
            // A pool moved meanwhile: looked at again in the next snapshot.
            self.gap_looked = None;
            return;
        }
        crate::diag::record(crate::diag::Record::PastTheGap {
            adopted: adopted.len() as u32,
            change_used,
            change_made,
        });
        // The pools as they are now, in the store (its own read of our addresses, BUG-194).
        self.chain.save_pools(wallet).await;
        let from = paid
            .iter()
            .filter(|(a, _)| adopted.contains(a))
            .map(|(_, height)| *height)
            .filter(|h| *h > 0)
            .min();
        if let Some(from) = from {
            super::note_past_the_gap(&mut self.past_the_gap_from, id, from.saturating_sub(1));
        }
    }

    /// BUG-232 review r1 M1: once the rescan for addresses taken in past the gap is done, the
    /// coins it gave (outputs once read as paid out) to the store: platform-wallet writes coins
    /// only from new records, and these came from records the wallet had.
    async fn save_past_the_gap_coins(&mut self, id: WalletId) {
        let Some(wallet) = self.chain.wallet(&id).await else {
            return;
        };
        let (coins, uncredited) = {
            let state = wallet.state().await;
            let all = self.merged_records(&state.core_wallet);
            (
                convert::coins_of_sent_outputs(&state.core_wallet, &all),
                convert::uncredited_outputs(&state.core_wallet, &all),
            )
        };
        crate::diag::record(crate::diag::Record::PastTheGapCoins {
            coins: coins.len() as u32,
        });
        self.chain.save_coins(id, coins).await;
        // Review r2: those still with no coin are asked once more after the next start, then
        // left; the others are done.
        let uncredited: Vec<_> = uncredited.into_iter().map(|(o, _)| o).collect();
        let (none, got) = super::settle_gap_pending(&mut self.gap_pending, &id, &uncredited);
        let keys =
            |v: Vec<key_wallet::dashcore::OutPoint>| v.iter().map(gap_key).collect::<Vec<_>>();
        self.gap_scans.finished(&keys(got), true);
        self.gap_scans.finished(&keys(none), false);
    }

    /// Rescans owed for payments that lost their block, and the lines waiting on one.
    async fn settle_rescans(
        &mut self,
        id: WalletId,
        birth_height: u32,
        synced: u32,
        lost_block: Option<mined::Lost>,
        contact_accounts: &[(String, Option<u32>)],
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
            if done.past_the_gap {
                self.save_past_the_gap_coins(id).await;
            }
            if let Some((asked, coins)) = done.contacts_asked {
                // BUG-221: found = DashPay coins it added; none, and it is asked once more after
                // the next start (`ContactScans::finished`).
                let now = self.dashpay_coins(&id).await;
                let found = now.saturating_sub(coins);
                self.contact_scans.finished(&done.contacts, found > 0);
                crate::diag::record(crate::diag::Record::RescanDone {
                    accounts: done.contacts.len() as u32,
                    found: found as u32,
                    secs: asked.elapsed().as_secs(),
                });
                // BUG-221: the coins by kind once it is done, counts only (QA reads dashpay).
                if let Some(wallet) = self.chain.wallet(&id).await {
                    let state = wallet.state().await;
                    crate::diag::record(convert::coins_record("after_rescan", &state.core_wallet));
                }
            }
        }
        // Read once, before a rescan is asked: a rescan asked below is owed in this snapshot.
        let status = self.chain.status();
        let sync_done = status.phase == sync::Phase::Synced;
        // TECH-spv-prune: the stored masternode messages dash-spv pruned, once synced.
        if sync_done && let Some(count) = self.chain.masternodes_pruned_since() {
            crate::diag::record(crate::diag::Record::MnPruned { count });
        }
        // Review r1 S2: the quorums' statuses once synced, for QA to compare with the start's.
        if sync_done
            && let Some((verified, unverified)) = self.chain.masternode_quorums_once_synced()
        {
            crate::diag::record(crate::diag::Record::MnQuorums {
                verified,
                unverified,
            });
        }
        // BUG-221: owed until the wallet's filters are at the tip, not at its height now.
        let up_to = super::rescan_end(synced, &status);
        if let Some(lost) = lost_block {
            let from = lost.from.saturating_sub(1);
            let why = "lost_block";
            turn::timed(
                "rescan_lost",
                self.rescan_from(id, from, why, up_to, Vec::new(), lost.txids),
            )
            .await;
        } else {
            let stale = self.mempool.not_rescanned(history);
            if !stale.is_empty() {
                turn::timed(
                    "rescan_stale",
                    self.rescan_from(id, birth_height, "stale", up_to, stale, Vec::new()),
                )
                .await;
            }
        }
        // BUG-150: a contact's receiving account built after the filters went past its coins: once
        // each, from the birth, not again while one for it is under way.
        let under_way = OwedRescan::contacts_under_way(&self.rescan_owed, &id).to_vec();
        let keys: Vec<String> = contact_accounts.iter().map(|(k, _)| k.clone()).collect();
        let owed: Vec<String> = self
            .contact_scans
            .owed(&keys)
            .into_iter()
            .filter(|a| !under_way.contains(a))
            .collect();
        // BUG-232 (review M1): the rescan owed to addresses taken in past the gap, once synced.
        let past_the_gap = super::take_past_the_gap(&mut self.past_the_gap_from, &id, sync_done);
        if !owed.is_empty() {
            // SYNC-7: from where the owed contacts begin, not the birth (35 s on testnet's
            // qa wallet, and cut short by a close): the birth only for one not established.
            // BUG-232 (review C2): one rescan for both, from the lower height.
            let from = rescan_start(contact_accounts, &owed, birth_height);
            let from = past_the_gap.map_or(from, |gap| gap.min(from));
            let coins = self.dashpay_coins(&id).await;
            let asked = turn::timed(
                "rescan_contacts",
                self.rescan_for_contacts(id, from, up_to, owed, coins),
            )
            .await;
            // Review r1 S2: not asked, it stays owed.
            match (asked, past_the_gap) {
                (false, Some(gap)) => {
                    super::note_past_the_gap(&mut self.past_the_gap_from, id, gap)
                }
                (true, Some(_)) => OwedRescan::for_past_the_gap(&mut self.rescan_owed, &id),
                _ => {}
            }
        } else if let Some(from) = past_the_gap {
            turn::timed(
                "rescan_past_the_gap",
                self.rescan_past_the_gap(id, from, synced, up_to),
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
    pub(in crate::real) async fn put_notes_on(
        &self,
        history: &mut [Activity],
    ) -> Option<NotesError> {
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
                    line.tax_transfer = notes.transfer(&line.key);
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
        fees_from(&self.chain, &self.journal, coins).await
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
                chain::stored_own(&store, &id),
            )
        })
        .await;
        if let Ok((records, consumed, own)) = read {
            if let Ok(records) = records {
                self.stored_history = records;
            }
            if let Ok(own) = own {
                self.stored_own = own;
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
        // The Platform journal is the credits' history (BUG-137).
        let working = self.work(PlatformWork::Credits);
        tokio::spawn(async move {
            // Lowered on every way out, a panic in the read too.
            struct Done(Arc<AtomicBool>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Relaxed);
                }
            }
            let _done = Done(busy);
            let _working = working;
            let Some(wallet) = chain.wallet(&id).await else {
                return;
            };
            let marks = platform_history::Marks::load(&marks_path);
            // SYNC-5: the names the journal has, so the identity search's are not asked again.
            let named: std::collections::HashSet<(String, String)> = journal
                .entries()
                .into_iter()
                .filter_map(|e| match (e.what, e.identity) {
                    (journal::What::Name { label }, Some(identity)) => Some((identity, label)),
                    _ => None,
                })
                .collect();
            let started = std::time::Instant::now();
            let read = platform_history::read(&chain, &wallet, id, &marks, &names, &named);
            match turn::timed("platform_history_read", read).await {
                // A lock (or a new lookup) since the start: this answer is for no one.
                Ok(_) if current.load(Ordering::Relaxed) != run => {}
                Ok(pass) => {
                    crate::diag::record(crate::diag::Record::HistoryPass {
                        identities: pass.identities,
                        requests: pass.requests,
                        ms: started.elapsed().as_millis() as u64,
                        failed: pass.failed,
                    });
                    let (entries, next) = (pass.entries, pass.marks);
                    // The entries first, then the marks: a crash between them reads again,
                    // and the keys drop the repeat. Not kept (the file's failure is in the
                    // journal already): the marks stay where they were, the next pass reads
                    // the same again.
                    if let Ok(changed) = journal.merge_network(entries) {
                        if pass.failed == 0 {
                            crate::diag::fresh("history", "network");
                        }
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
#[allow(clippy::too_many_arguments)] // the round's handles, each its own Arc (BUG-147 budgets, works)
pub(super) async fn keep_balances_fresh(
    chain: Arc<Chain>,
    id: WalletId,
    live: impl Fn() -> bool,
    stale: Arc<tokio::sync::Notify>,
    checked: Arc<AtomicBool>,
    refresh: impl Fn(),
    budgets: ops::app_connect2::AppBudgets,
    works: Arc<super::works::Works>,
) {
    let mut round = 0u32;
    // BUG-147: the credits and the journal's length at the last round, and when the connected
    // apps' budgets were read last.
    let mut seen = (None, budgets.journal.count());
    let mut budgets_at: Option<std::time::Instant> = None;
    while live() {
        let all = round.is_multiple_of(BALANCES_ALL_EVERY);
        round = round.wrapping_add(1);
        if let Some(wallet) = chain.wallet(&id).await {
            // The read, not the wait for the next one (BUG-137).
            let working = works.start(PlatformWork::Credits);
            let result =
                turn::timed("refresh_balances", chain.refresh_balances(&wallet, all)).await;
            drop(working);
            if round_reads_all(all, &result) && live() {
                checked.store(true, Ordering::Relaxed);
                crate::diag::fresh("balances", "network");
            }
            if result.is_ok() {
                let now = (
                    chain.first_identity_credits(&wallet, false).await,
                    budgets.journal.count(),
                );
                let dropped = dropped_unexplained(seen, now);
                seen = now;
                if budgets_due(dropped, budgets_at.map(|at| at.elapsed())) {
                    budgets.read_all(&chain).await;
                    budgets_at = Some(std::time::Instant::now());
                }
                refresh();
            }
        }
        tokio::select! {
            _ = stale.notified() => {}
            _ = tokio::time::sleep(BALANCES_EVERY) => {}
        }
    }
}

/// How often the connected apps' budgets are read while the wallet is open (BUG-147).
const BUDGETS_EVERY: Duration = Duration::from_secs(600);

/// Whether the shown identity's credits went down between two rounds with no new journal line to
/// say why (BUG-147): an app spent from its keys' budget, most likely. `(credits, journal length)`.
fn dropped_unexplained(before: (Option<u64>, usize), now: (Option<u64>, usize)) -> bool {
    matches!((before.0, now.0), (Some(was), Some(is)) if is < was) && now.1 == before.1
}

/// Whether a round reads the connected apps' budgets (BUG-147): the credits went down unexplained,
/// or the last read is [`BUDGETS_EVERY`] old, or there was none in this unlock.
fn budgets_due(dropped: bool, since: Option<Duration>) -> bool {
    dropped || since.is_none_or(|d| d >= BUDGETS_EVERY)
}

#[cfg(test)]
mod budgets_due_tests {
    use super::*;

    /// BUG-147 AC-T2: credits down with no journal line read the budgets at once; down with a
    /// line (evolite's own write) or up do not; otherwise once at the start and every 10 minutes.
    #[test]
    fn the_budgets_are_read_when_credits_fall_unexplained() {
        assert!(dropped_unexplained((Some(1_000), 5), (Some(900), 5)));
        assert!(!dropped_unexplained((Some(1_000), 5), (Some(900), 6)));
        assert!(!dropped_unexplained((Some(1_000), 5), (Some(1_100), 5)));
        assert!(!dropped_unexplained((None, 5), (Some(900), 5)));
        assert!(!dropped_unexplained((Some(1_000), 5), (None, 5)));

        let minute = Some(Duration::from_secs(60));
        assert!(budgets_due(false, None));
        assert!(!budgets_due(false, minute));
        assert!(budgets_due(true, minute));
        assert!(budgets_due(false, Some(BUDGETS_EVERY)));
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

/// Minutes with no new block worth saying (SPV item 2a): 30 and more.
const STALE_AFTER: u64 = 30;

/// How long the wallet must be synced, with the tip not grown once since the start, before the
/// header's age alone says (SPV 2a, review r1 Should 1): an honest network brings a block well
/// within it.
const NO_GROWTH_SINCE_START: u64 = 10 * 60;

/// The tip on this run's steady clock, as `Real::tip_grew` keeps it (SPV item 2a).
pub(in crate::real) struct TipClock {
    /// The highest tip seen on this run; 0 before the first.
    tip: u32,
    /// When it grew, or when the first tip was seen.
    grew_at: std::time::Instant,
    /// The tip grew past the first one seen on this run.
    grew_this_run: bool,
    /// Since when the wallet has been synced without a break.
    synced_at: Option<std::time::Instant>,
}

impl TipClock {
    pub(in crate::real) fn new() -> Self {
        Self {
            tip: 0,
            grew_at: std::time::Instant::now(),
            grew_this_run: false,
            synced_at: None,
        }
    }
}

/// What `Real::tip_clock` reads off the steady clock, in seconds.
struct TipClockNow {
    since_grew: u64,
    grew_this_run: bool,
    synced_for: Option<u64>,
}

/// The growth side of `stale_minutes`, in seconds of this run's steady clock.
#[derive(Clone, Copy)]
pub(in crate::real) struct TipGrowth {
    /// How long the tip has not grown (since the start, if it never did).
    since_grew: u64,
    /// The tip grew at least once on this run.
    grew_this_run: bool,
    /// How long the wallet has been synced.
    synced_for: u64,
}

/// SPV item 2a: minutes with no new block, when both clocks say so: the tip header's own time
/// against the computer's (`now - tip_time`), and how long the tip has not grown on this run's
/// steady clock. The smaller of the two; `None` under 30 minutes or without the header's time.
/// A computer clock a day ahead with a fresh tip says nothing: the tip grew a minute ago
/// (AC-T2а.1).
///
/// After a restart the steady clock knows nothing of the time before it (review r1 Should 1): once
/// the wallet has been synced for 10 minutes and the tip has not grown once on this run, the
/// header's age alone says. The cost, accepted: a clock far ahead and a 10-minute pause in the
/// blocks right after the start warn falsely.
pub(in crate::real) fn stale_minutes(
    growth: TipGrowth,
    tip_time: Option<i64>,
    now: evolite_api::Timestamp,
) -> Option<u32> {
    let by_header = now.saturating_sub(tip_time?).max(0) as u64 / 60;
    let minutes = if !growth.grew_this_run && growth.synced_for >= NO_GROWTH_SINCE_START {
        by_header
    } else {
        by_header.min(growth.since_grew / 60)
    };
    (minutes >= STALE_AFTER).then(|| minutes.min(u32::MAX as u64) as u32)
}

#[cfg(test)]
mod stale_tests {
    use super::{TipGrowth, stale_minutes};

    const MIN: i64 = 60;

    /// The tip grew on this run, `since` seconds ago; synced long since.
    fn grew(since: u64) -> TipGrowth {
        TipGrowth {
            since_grew: since,
            grew_this_run: true,
            synced_for: 86_400,
        }
    }

    /// Since the start the tip has not grown; synced for `synced` seconds.
    fn never(synced: u64) -> TipGrowth {
        TipGrowth {
            since_grew: synced,
            grew_this_run: false,
            synced_for: synced,
        }
    }

    /// AC-T2а.1: the thresholds by both clocks; the smaller says; a clock gone ahead by a day
    /// while the tip keeps growing says nothing; no header time, nothing.
    #[test]
    fn a_tip_is_stale_only_by_both_clocks() {
        let now = 1_800_000_000;
        // 34 minutes by both: «34 min ago».
        assert_eq!(
            stale_minutes(grew(34 * 60), Some(now - 34 * MIN), now),
            Some(34)
        );
        // 29 minutes: still fresh.
        assert_eq!(
            stale_minutes(grew(29 * 60), Some(now - 29 * MIN), now),
            None
        );
        // Over an hour by both: 125 minutes, the smaller of the two.
        assert_eq!(
            stale_minutes(grew(130 * 60), Some(now - 125 * MIN), now),
            Some(125)
        );
        // The computer's clock a day ahead, the tip grew a minute ago: nothing.
        assert_eq!(
            stale_minutes(grew(60), Some(now - 24 * 60 * MIN), now),
            None
        );
        // The tip has not grown here for 40 minutes, but its header is 5 minutes old (a node
        // that was behind caught up with an old block, or a clock behind): nothing.
        assert_eq!(stale_minutes(grew(40 * 60), Some(now - 5 * MIN), now), None);
        // A header from the future (a clock behind): nothing.
        assert_eq!(
            stale_minutes(grew(40 * 60), Some(now + 10 * MIN), now),
            None
        );
        assert_eq!(
            stale_minutes(grew(3_600), None, now),
            None,
            "no header time"
        );
    }

    /// Review r1 Should 1: after a restart the steady clock starts anew; once synced for 10
    /// minutes with the tip never grown, the header's age says alone. Once the tip has grown,
    /// both clocks again.
    #[test]
    fn after_a_restart_the_header_says_once_synced_for_ten_minutes() {
        let now = 1_800_000_000;
        // 10 minutes synced, no growth, a header 5 hours old: 300.
        assert_eq!(
            stale_minutes(never(600), Some(now - 300 * MIN), now),
            Some(300)
        );
        // 9 minutes: not yet.
        assert_eq!(stale_minutes(never(540), Some(now - 300 * MIN), now), None);
        // The tip grew 3 minutes ago, the header 5 hours old: nothing.
        assert_eq!(stale_minutes(grew(180), Some(now - 300 * MIN), now), None);
        // The documented cost: a clock a day ahead, 10 minutes without a block right after the
        // start: a warning.
        assert_eq!(
            stale_minutes(never(600), Some(now - 24 * 60 * MIN), now),
            Some(24 * 60)
        );
    }
}

/// When the tip last grew (BUG-155 AC-A9), kept in `seen` (the highest tip and when it came):
/// a higher `tip` is new at `now`; the same or a lower one (a reorg, a node behind) keeps the
/// time. The first tip seen (from the store, at the start) is no new block:
/// it has no time until the tip grows past it (review r1, P3 1).
fn tip_seen(
    seen: &mut (u32, Option<evolite_api::Timestamp>),
    tip: u32,
    now: evolite_api::Timestamp,
) -> Option<evolite_api::Timestamp> {
    if seen.0 == 0 {
        seen.0 = tip;
    } else if tip > seen.0 {
        *seen = (tip, Some(now));
    }
    seen.1
}

#[cfg(test)]
mod tip_seen_tests {
    use super::tip_seen;

    /// BUG-155 AC-A9: the time is the new tip's; it stays while the tip does, and a node behind
    /// does not move it back.
    #[test]
    fn the_time_is_when_the_tip_grew() {
        let mut seen = (0, None);
        assert_eq!(tip_seen(&mut seen, 0, 100), None, "no tip yet");
        assert_eq!(
            tip_seen(&mut seen, 10, 200),
            None,
            "the first tip: from the store"
        );
        assert_eq!(tip_seen(&mut seen, 11, 300), Some(300), "a new block");
        assert_eq!(tip_seen(&mut seen, 11, 400), Some(300), "the same tip");
        assert_eq!(tip_seen(&mut seen, 9, 500), Some(300), "a node behind");
        assert_eq!(tip_seen(&mut seen, 12, 600), Some(600));
    }
}

/// TECH-signing-identity AC-T1, AC-T2: the core's paying tasks read the credits before and after
/// of the identity they act as ([`acting`], once), never of whoever is shown by then.
#[cfg(test)]
mod signing_tests {
    #[test]
    fn paid_tasks_read_the_credits_of_the_identity_they_act_as() {
        let ops = [
            include_str!("ops/identity.rs"),
            include_str!("ops/dashpay.rs"),
            include_str!("ops/tokens.rs"),
            include_str!("ops/token_actions.rs"),
            include_str!("ops/token_manage.rs"),
            include_str!("ops/identity_keys.rs"),
            include_str!("ops/app_connect2.rs"),
            include_str!("ops/app_jobs.rs"),
            include_str!("ops/pool.rs"),
        ];
        let shown_credits = ["first_identity", "_credits("].concat();
        for source in ops {
            assert!(
                !source.contains(&shown_credits),
                "credits of whoever is shown"
            );
        }
    }
}

/// BUG-206: every paid write that acts «as the identity» takes it when its command came
/// ([`Real::shown_now`] before the task) and never reads «Act as» again in its task.
/// Mutant: a task reading [`acting_identity`] (or the old per-task read) again fails here.
#[cfg(test)]
pub(in crate::real) mod pinned_tests {
    /// The source of `name`'s function in `source`, up to the next function of the file.
    pub(in crate::real) fn body<'a>(source: &'a str, name: &str) -> &'a str {
        let start = source
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("no fn {name}"));
        let rest = &source[start + 3..];
        let end = [
            "\n    pub(in crate::real) ",
            "\n    fn ",
            "\n    async fn ",
            "\n}\n",
        ]
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
        &rest[..end]
    }

    #[test]
    fn paid_writes_act_as_the_identity_their_command_came_with() {
        let files = [
            (
                include_str!("ops/identity.rs"),
                &[
                    "top_up_identity",
                    "add_name",
                    "claim_name",
                    "set_name_link",
                    "send_credits",
                ][..],
            ),
            (
                include_str!("ops/dashpay.rs"),
                &["contact_job", "save_profile"][..],
            ),
            (include_str!("ops/tokens.rs"), &["send_token"][..]),
            (
                include_str!("ops/token_actions.rs"),
                &["claim_token", "buy_token"][..],
            ),
            (include_str!("ops/token_manage.rs"), &["issuer_action"][..]),
            (
                include_str!("ops/pool.rs"),
                &["shield", "top_up_from_pool"][..],
            ),
            (
                include_str!("ops/core.rs"),
                &["preview_payment", "send"][..],
            ),
        ];
        for (source, names) in files {
            for name in names {
                let f = body(source, name);
                assert!(f.contains("self.shown_now(id)"), "{name}: not pinned");
                assert!(
                    f.contains("pinned_acting(&wallet, pinned)"),
                    "{name}: the pin not used"
                );
                assert!(
                    !f.contains("acting_identity("),
                    "{name}: reads «Act as» again"
                );
            }
        }
    }
}

/// An output's name in `gap-rescans.json` (BUG-232 review r2): `<txid>:<vout>`.
fn gap_key(outpoint: &key_wallet::dashcore::OutPoint) -> String {
    format!("{}:{}", outpoint.txid, outpoint.vout)
}

#[cfg(test)]
mod shown_pin_tests {
    use super::*;

    fn b58(n: u8) -> String {
        Identifier::new([n; 32]).to_string(Encoding::Base58)
    }

    /// BUG-206 r1 Must 1: the command acts as the identity the screen showed, not the raw choice.
    /// Right after an unlock the chosen identity (slot 2) has not loaded: the screen shows and
    /// the person confirms the lowest (slot 0); once slot 2 loads the task still pays as slot 0.
    /// And an identity shown that is gone by the task is a refusal, not the lowest in its place.
    /// Mutant: resolving the choice (as r1 did) pays as slot 2 in the first case and as slot 0 in
    /// the second.
    #[test]
    fn a_command_acts_as_the_identity_shown_and_refuses_one_gone() {
        let shown = Some(Identifier::new([1; 32]));
        let later = [(b58(1), 0), (b58(3), 2)];
        assert_eq!(pinned_in(shown, later.clone().into_iter()), shown);
        let gone = [(b58(2), 1), (b58(3), 2)];
        assert_eq!(pinned_in(shown, gone.into_iter()), None);
        assert_eq!(
            pinned_in(None, later.into_iter()),
            None,
            "none shown, none acts"
        );
    }
}

/// [`Real::fees_now`] from the chain and the journal alone, for a task that outlives the call
/// (BUG-236: App Connect's price is the same as every other form's, not the defaults).
pub(super) async fn fees_from(chain: &Chain, journal: &journal::Journal, coins: usize) -> Fees {
    let mut fees = fees(coins);
    let mainnet = crate::network::current() == evolite_api::Network::Mainnet;
    if let Ok(sdk) = chain.sdk().await {
        pool::set_fees(&mut fees, sdk.version());
        platform_fees(&mut fees, sdk.version(), journal);
        evolite_chain::convert::contest_times(&mut fees, sdk.version(), mainnet);
        fees.lock_inputs_max = evolite_chain::convert::lock_inputs_max(sdk.version());
    } else {
        // A name contest's durations before the SDK is up: the latest protocol's (FEAT-0021).
        let latest = dpp::version::PlatformVersion::latest();
        evolite_chain::convert::contest_times(&mut fees, latest, mainnet);
    }
    fees
}
