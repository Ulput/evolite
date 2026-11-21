//! What dash-spv and key-wallet report, in the API's terms: sync stages, history, fees, and
//! the one address check before paying.

use evolite_api::*;
use key_wallet::dashcore::address::{Address, AddressType, NetworkUnchecked};
use key_wallet::managed_account::transaction_record::{
    OutputRole, TransactionDirection, TransactionRecord,
};
use key_wallet::transaction_checking::TransactionContext;
use key_wallet::transaction_checking::transaction_router::TransactionType;
use key_wallet::wallet::managed_wallet_info::fee::{FeeRate, estimate_tx_size};

use super::sync::SyncStatus;

/// The wallet's coins are known: headers at the tip and, for an open wallet, its filters too —
/// once done up to the headers' tip in this run ([`SyncStatus::filters_at_tip`]), not only
/// "done", which dash-spv says right after a start against the last run's height (BUG-77 r1
/// must), and not below that height since. Once latched, dash-spv's "done" is not asked again:
/// it puts the filters back to syncing for a moment on each new block, and the balance dimmed
/// at every block (BUG-77 r2 should 1). The latch goes when the headers are not done (all peers
/// lost) and on a rewind ([`SyncStatus::latch_filters_at_tip`]).
/// Until the first filter report after a restart there is none, and that is "not yet".
pub fn coins_known(s: &SyncStatus, wallet: bool) -> bool {
    s.headers.is_some_and(|h| h.done)
        && (!wallet
            || match s.wallet_synced {
                // BUG-162: the wallet's own sync, not the client's filters — those may have
                // reached the tip for nobody before the wallet joined.
                Some(at) => s.wallet_at_tip.is_some_and(|tip| at >= tip),
                // Before the core's first note of it: the filters' latch (reset when a wallet
                // joins).
                None => s
                    .filters
                    .is_some_and(|f| s.filters_at_tip.is_some_and(|at| f.height >= at)),
            })
}

/// A wallet's balance counted between two reads of the sync status is checked only when its
/// coins were known at both: `before` is read before the coins are counted, `after` once they
/// are. A count made while a rewound pass was below the tip and checked only once the pass had
/// reached it showed 4,82 as checked for a moment (live, BUG-77 workarounds).
pub fn count_checked(counted: bool, before: &SyncStatus, after: &SyncStatus) -> bool {
    counted && coins_known(before, true) && coins_known(after, true)
}

/// What dash-spv reports, in the terms of the screens. `wallet`: a wallet is open, so its
/// filters are part of the sync.
pub fn sync_info(s: &SyncStatus, wallet: bool) -> SyncInfo {
    let headers = s.headers;
    let tip = headers.map_or(0, |h| h.target);
    // The part of `from..to` done at `at`: the work of this run, not the height. A warm folder
    // is near the tip from the start, and height / tip said «100 %» while there were hundreds
    // of blocks to go (BUG-60). Nothing between the two: nothing to count.
    let span = |from: u32, at: u32, to: u32| {
        (to > from).then(|| at.saturating_sub(from) as f32 / (to - from) as f32)
    };
    let (stage, progress) = match (headers, s.masternodes, s.filters) {
        // Nothing heard from the network yet (or it failed before that); the client may
        // still be reading what it saved.
        (None, _, _) => (SyncStage::Connecting, None),
        // dash-spv's own count: the joined-up tip and the segments downloaded past it.
        (Some(h), _, _) if !h.done => (
            SyncStage::Headers,
            span(
                s.headers_from.unwrap_or(0).min(h.height),
                h.height + s.headers_buffered,
                h.target,
            ),
        ),
        // The list height moves only when the lists are done: no count, only the step
        // (BUG-61).
        (_, Some(m), _) if !m.done => (
            if s.lists_answered {
                SyncStage::Quorums
            } else {
                SyncStage::Masternodes
            },
            None,
        ),
        // A third each for the filter headers, the filters downloaded and the batches done:
        // the done height alone stood at 28 % for minutes while a restored wallet's blocks
        // came, then jumped to the end.
        // Done only at the headers' tip (BUG-77 r2): what dash-spv calls done right after a start
        // is the last run's height, and the coins are not known yet.
        (_, _, Some(f)) if wallet && !coins_known(s, true) => {
            // No start known yet (the first report of a new or restored wallet): no count.
            let from = s.filters_from.map(|from| from.min(f.height));
            let stored = s.filters_stored.max(f.height);
            // The last part is the wallet's own pass when known (BUG-162), not the client's.
            let done = s.wallet_synced.unwrap_or(f.height);
            let parts = [s.filter_headers.max(stored), stored, done];
            (
                SyncStage::Filters,
                from.and_then(|from| {
                    parts
                        .iter()
                        .map(|&at| span(from, at, f.target))
                        .sum::<Option<f32>>()
                        .map(|sum| sum / parts.len() as f32)
                }),
            )
        }
        (_, _, None) if wallet => (SyncStage::Filters, None),
        // Waits for the network's next ChainLock: nothing to count.
        _ if s.chainlock_height.is_none() => (SyncStage::ChainLock, None),
        _ => (SyncStage::Done, Some(1.0)),
    };
    SyncInfo {
        stage,
        progress: progress.map(|p| p.clamp(0.0, 1.0)),
        tip_height: tip,
        headers_height: headers.map_or(0, |h| h.height),
        filters_height: s.filters.map_or(0, |f| f.height),
        core_peers: s.peers as u32,
        platform_nodes: 0,
        minutes_left: None,
        // A checked ChainLock near the tip: the history we show cannot be rewritten.
        chainlocked: s.chainlock_height.is_some_and(|c| tip > 0 && c + 10 >= tip),
        error: s.error.clone(),
        folder_refused: s.folder_refused.clone(),
        // The core counts the folder itself.
        disk_bytes: None,
        core_node: s.core_node.clone(),
        core_node_next: None,
        // The core asks the client and the SDK itself.
        core_nodes: Vec::new(),
        dapi_nodes: Vec::new(),
        platform: Vec::new(),
        // The core notes when the tip grew.
        tip_at: None,
        stale_minutes: None,
        contacts_rescan: false,
    }
}

/// A testnet Core address to pay to; a mainnet one is refused, not converted. Only P2PKH and
/// P2SH: a witness address parses too, but a payment to it could never be spent (R-3).
pub fn payee(to: &str) -> Result<Address<NetworkUnchecked>, CoreError> {
    to.trim()
        .parse::<Address<NetworkUnchecked>>()
        .ok()
        .filter(|a| a.is_valid_for_network(crate::network::dash()))
        .filter(|a| {
            matches!(
                a.assume_checked_ref().address_type(),
                Some(AddressType::P2pkh | AddressType::P2sh)
            )
        })
        .ok_or(CoreError::BadAddress)
}

/// Whether `amount` to `to` is below the dust limit: the network does not relay such an output, and
/// its inputs would stay reserved for nothing (R-4). `None`: an address not known yet, a
/// contact's, which is always P2PKH.
pub fn is_dust(to: Option<&Address>, amount: u64) -> bool {
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{PubkeyHash, ScriptBuf};
    let script = match to {
        Some(address) => address.script_pubkey(),
        None => ScriptBuf::new_p2pkh(&PubkeyHash::all_zeros()),
    };
    amount < script.dust_value().to_sat()
}

/// Why `to` is no address at all — a typo, a missing or extra character (its checksum does
/// not add up) — said apart from a good address of another network, which stays `BadAddress`.
pub fn not_an_address(to: &str) -> Option<CoreError> {
    to.trim()
        .parse::<Address<NetworkUnchecked>>()
        .is_err()
        .then(|| {
            CoreError::NotAvailable(
                tr!("This is not a Dash address: a typo or an extra character — the checksum does not match")
                    .into(),
            )
        })
}

/// Payments out to an address of a contact's key name the contact (`@name`, or a short id
/// while it has none), the address staying beside it for the card. `keys`: address → the
/// contact's identity.
pub fn name_contact_payments(
    history: &mut [Activity],
    keys: &std::collections::HashMap<String, (String, String)>,
    contacts: &[Contact],
) {
    for a in history {
        let ActivityKind::Sent { to } = &mut a.kind else {
            continue;
        };
        let Some((_, friend)) = keys.get(to.as_str()) else {
            continue;
        };
        let name = contacts
            .iter()
            .find(|c| c.identity_id == *friend)
            .and_then(|c| c.name.clone())
            .map(|n| format!("@{n}"))
            .unwrap_or_else(|| {
                let chars: Vec<char> = friend.chars().collect();
                let head: String = chars.iter().take(4).collect();
                let tail: String = chars[chars.len().saturating_sub(4)..].iter().collect();
                format!("{head}…{tail}")
            });
        a.address = Some(std::mem::replace(to, name));
    }
}

/// Payments to each of `me`'s contacts as the history shows them (after
/// [`name_contact_payments`]): a wallet restored from its phrase has none on record in
/// platform-wallet, the chain has them. `keys`: each contact key's address → (our identity, the
/// contact). BUG-196: by the pair — the history and the keys are the whole wallet's, and its
/// identities may share a contact; one's payments are not another's.
pub fn count_contact_payments(
    history: &[Activity],
    keys: &std::collections::HashMap<String, (String, String)>,
    me: Option<&str>,
    contacts: &mut [Contact],
) {
    let Some(me) = me else {
        return;
    };
    for c in contacts
        .iter_mut()
        .filter(|c| c.state == ContactState::Established)
    {
        let to_them = history.iter().filter(|a| {
            matches!(a.kind, ActivityKind::Sent { .. })
                && a.address
                    .as_ref()
                    .and_then(|ad| keys.get(ad))
                    .is_some_and(|(ours, friend)| ours == me && *friend == c.identity_id)
        });
        let (count, last) =
            to_them.fold((0u32, None), |(n, last), a| (n + 1, last.max(Some(a.at))));
        c.payments = c.payments.max(count);
        c.last_payment = c.last_payment.max(last);
    }
}

/// A ChainLock on block `chainlocked` locks every block under it too: payments in those blocks
/// are as final as the locked one, whatever key-wallet recorded when it saw them (a record
/// stored before the lock came, or read back after a restart, stays "in a block").
pub fn settle(history: &mut [Activity], chainlocked: Option<u32>) {
    let Some(locked) = chainlocked else {
        return;
    };
    for a in history {
        if a.proof == Proof::InBlock && a.block.is_some_and(|b| b <= locked) {
            a.proof = Proof::ChainLock;
            // The card says it in words too (FEAT-0005): the block it is in.
            if let Some(detail) = a.detail.as_deref_mut() {
                detail.confirm = confirm(a.proof, a.block);
            }
        }
    }
}

/// How final a transaction is. dash-spv puts it in `InstantSend` on any InstantLock a peer sends,
/// verified or not (R-1): only a lock whose signature was verified (`instant_verified`) makes
/// it InstantSend here; otherwise it is still in the mempool.
pub fn proof(context: &TransactionContext, instant_verified: bool) -> Proof {
    match context {
        TransactionContext::Mempool => Proof::Mempool,
        TransactionContext::InstantSend(_) if instant_verified => Proof::InstantLock,
        TransactionContext::InstantSend(_) => Proof::Mempool,
        TransactionContext::InBlock(_) => Proof::InBlock,
        TransactionContext::InChainLockedBlock(_) => Proof::ChainLock,
    }
}

/// What a Core transaction was, from the wallet's side: an asset lock moves DASH to Platform
/// (an identity's credits or a Platform address), an asset unlock brings withdrawn credits back;
/// a payment out names the address it went to.
fn kind_of(
    direction: TransactionDirection,
    kind: TransactionType,
    sent_to: Option<String>,
) -> (ActivityKind, Chain) {
    match (kind, direction) {
        (TransactionType::AssetLock, _) => (
            ActivityKind::TopUp {
                identity: String::new(),
                creates: false,
                credits: evolite_api::Credits::Unknown,
            },
            Chain::CoreToPlatform,
        ),
        (TransactionType::AssetUnlock, _) => (
            ActivityKind::Received {
                from: Some(tr!("withdrawal from Platform").into()),
            },
            Chain::Core,
        ),
        (_, TransactionDirection::Incoming) => (ActivityKind::Received { from: None }, Chain::Core),
        (_, TransactionDirection::Outgoing) => (
            ActivityKind::Sent {
                to: sent_to.unwrap_or_default(),
            },
            Chain::Core,
        ),
        (_, TransactionDirection::Internal | TransactionDirection::CoinJoin) => {
            (ActivityKind::SelfTransfer { moved: 0 }, Chain::Core)
        }
    }
}

/// What a transaction did to the wallet, each input and each output once: our outputs (received,
/// change) less the coins of ours it spent, by their index in the transaction. platform-wallet
/// folds a transaction's records from every account it touches and adds up their nets; a coin
/// two accounts hold (a contact's coin also in account 0, BUG-77 U1) was then subtracted twice —
/// a transfer to an own address showed −0,05153778 for a fee of 0,00003778 (BUG-85). The
/// details are already one per index (the fold keeps the first); key-wallet's own net is this
/// same sum per account (`recompute_net_and_direction`). A payment in is counted by its details
/// too: one transaction paying two of our accounts has both outputs in them (this run's records
/// as one, [`one_per_transaction`]), and an output two accounts hold would be added twice by the
/// fold (BUG-85 r1 should 1). A record keeps its own net only with no details at all, and an
/// asset lock with no input of ours: its details there are of the lock's payload.
pub fn net_once(record: &TransactionRecord) -> i64 {
    use std::collections::BTreeMap;
    let spent: BTreeMap<u32, u64> = record
        .input_details
        .iter()
        .map(|d| (d.index, d.value))
        .collect();
    let owned: BTreeMap<u32, u64> = record
        .output_details
        .iter()
        .filter(|o| matches!(o.role, OutputRole::Received | OutputRole::Change))
        .map(|o| (o.index, o.value))
        .collect();
    if spent.is_empty()
        && (record.output_details.is_empty()
            || record.transaction_type == TransactionType::AssetLock)
    {
        return record.net_amount;
    }
    owned.values().sum::<u64>() as i64 - spent.values().sum::<u64>() as i64
}

/// This run's records (one per account a transaction touches) as one per transaction, each
/// group folded with [`merge_slices`] in the order given. Picking one account's record left a
/// transfer to an own address that spent a contact's coin as the contact account's view: the
/// coin out, nothing back (BUG-85).
pub fn one_per_transaction<'a>(
    records: impl IntoIterator<Item = &'a TransactionRecord>,
) -> Vec<TransactionRecord> {
    let mut merged: Vec<TransactionRecord> = Vec::new();
    // BUG-239: a contact's watch account (the addresses we pay them at) sees the contact's
    // coins as received and the contact's spends as its inputs: never ours. platform-wallet
    // leaves these slices out of the store; this run's records leave them out too.
    for r in records.into_iter().filter(|r| !is_contact_watch(r)) {
        match merged.iter_mut().find(|m| m.txid == r.txid) {
            Some(m) => *m = merge_slices(m, r),
            None => merged.push(r.clone()),
        }
    }
    merged
}

/// How final a context is: ChainLock over a block, over an InstantLock, over the mempool.
fn finality(context: &TransactionContext) -> u8 {
    match context {
        TransactionContext::Mempool => 0,
        TransactionContext::InstantSend(_) => 1,
        TransactionContext::InBlock(_) => 2,
        TransactionContext::InChainLockedBlock(_) => 3,
    }
}

/// Two accounts' records of one transaction as one (BUG-237: the store keeps one row per
/// transaction, and a later account's slice wrote over the others' — a DashPay rescan's left a
/// payment without its change). The same rule for the store's write and the screen's merge.
///
/// - The base, and so `account_type`, is the slice with inputs — the spending account's, which
///   must know the transaction after a restart (the store puts a record into its account). When
///   both or neither have inputs, `first` (the stored one).
/// - The context is the more final one (ChainLock > block > InstantLock > mempool).
/// - An asset lock stays its base alone: its other account's details are of the lock's payload.
/// - Otherwise the inputs and outputs of both, once by index; an output one slice owns and the
///   other sees as `Sent` stays owned. Net, direction and fee follow from them (key-wallet's
///   rules) — only when something was added, so a slice merged with itself is unchanged.
pub fn merge_slices(first: &TransactionRecord, second: &TransactionRecord) -> TransactionRecord {
    let (base, other) = if first.input_details.is_empty() && !second.input_details.is_empty() {
        (second, first)
    } else {
        (first, second)
    };
    let mut merged = base.clone();
    if finality(&other.context) > finality(&merged.context) {
        merged.context = other.context.clone();
    }
    if merged.transaction_type == TransactionType::AssetLock
        || other.transaction_type == TransactionType::AssetLock
    {
        return merged;
    }
    let owned = |role: &OutputRole| matches!(role, OutputRole::Received | OutputRole::Change);
    let mut added = false;
    for d in &other.input_details {
        if !merged.input_details.iter().any(|m| m.index == d.index) {
            merged.input_details.push(d.clone());
            added = true;
        }
    }
    for d in &other.output_details {
        match merged
            .output_details
            .iter_mut()
            .find(|m| m.index == d.index)
        {
            Some(m) if owned(&d.role) && !owned(&m.role) => {
                *m = d.clone();
                added = true;
            }
            Some(_) => {}
            None => {
                merged.output_details.push(d.clone());
                added = true;
            }
        }
    }
    if added {
        merged.input_details.sort_by_key(|d| d.index);
        merged.output_details.sort_by_key(|d| d.index);
        settle_record(&mut merged);
    }
    merged
}

/// BUG-239: the record of a contact's watch account (`DashpayExternalAccount`, the addresses
/// we pay a contact at): its «received» and its inputs are the contact's, not the wallet's.
pub fn is_contact_watch(record: &TransactionRecord) -> bool {
    matches!(
        record.account_type,
        key_wallet::account::AccountType::DashpayExternalAccount { .. }
    )
}

/// A record's net, direction and fee again from its details, by key-wallet's rules: no `Sent`
/// output, inputs and outputs of ours — a transfer between own addresses; inputs of ours — out;
/// else in. With every input ours, the fee is what they held over what the outputs carry.
fn settle_record(record: &mut TransactionRecord) {
    record.net_amount = net_once(record);
    let sent = record
        .output_details
        .iter()
        .any(|d| d.role == OutputRole::Sent);
    let ours = record
        .output_details
        .iter()
        .any(|d| matches!(d.role, OutputRole::Received | OutputRole::Change));
    let inputs = !record.input_details.is_empty();
    record.direction = if record.transaction_type == TransactionType::CoinJoin {
        TransactionDirection::CoinJoin
    } else if !sent && inputs && ours {
        TransactionDirection::Internal
    } else if inputs {
        TransactionDirection::Outgoing
    } else {
        TransactionDirection::Incoming
    };
    if record.input_details.len() == record.transaction.input.len() {
        let spent: u64 = record.input_details.iter().map(|d| d.value).sum();
        let out: u64 = record.transaction.output.iter().map(|o| o.value).sum();
        record.fee = spent.checked_sub(out).or(record.fee);
    }
}

/// A stored record's outputs to an address of this wallet as ours (BUG-86). The store keeps one
/// row per transaction and the last write wins: a transfer of «All» to an own address of
/// account 0 that also spent contact coins was kept as the contact account's slice alone —
/// its coins in, the output `Sent`, because that account does not know account 0's address
/// (the rescan after the sync recorded it there and wrote it over account 0's record).
/// [`attribute_late_inputs`] then gave it the other inputs, and it showed as paid out in full,
/// −3,86574464. `ours` says what an address is to the wallet (`Received` or `Change`) over
/// every account that holds coins. An asset lock is left as it is (its details there are of
/// the lock's payload). True when an output changed; the net, direction and fee follow.
pub fn own_outputs(
    record: &mut TransactionRecord,
    ours: impl Fn(&Address) -> Option<OutputRole>,
) -> bool {
    if record.transaction_type == TransactionType::AssetLock {
        return false;
    }
    let mut changed = false;
    for d in &mut record.output_details {
        if d.role != OutputRole::Sent {
            continue;
        }
        if let Some(role) = d.address.as_ref().and_then(&ours) {
            d.role = role;
            changed = true;
        }
    }
    if changed {
        settle_record(record);
    }
    changed
}

/// The index of each asset unlock among `records`, by txid: what Platform wrote into the
/// withdrawal it pays (FEAT-0005 АРХ-2).
pub fn asset_unlocks(records: &[TransactionRecord]) -> std::collections::HashMap<String, u64> {
    records
        .iter()
        .filter(|r| r.transaction_type == TransactionType::AssetUnlock)
        .filter_map(|r| unlock_index(&r.transaction).map(|index| (r.txid.to_string(), index)))
        .collect()
}

fn unlock_index(tx: &key_wallet::dashcore::Transaction) -> Option<u64> {
    use key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
    match &tx.special_transaction_payload {
        Some(TransactionPayload::AssetUnlockPayloadType(p)) => Some(p.base.index),
        _ => None,
    }
}

/// `instant_verified`: the InstantLock of the transaction was verified ([`proof`]).
pub fn activity(record: &TransactionRecord, seen: Timestamp, instant_verified: bool) -> Activity {
    let net = net_once(record);
    let sent_to = record
        .output_details
        .iter()
        .find(|o| o.role == OutputRole::Sent)
        .and_then(|o| o.address.as_ref())
        .map(|a| a.to_string());
    // Our own receiving address the payment came to, and what came there.
    let received = record
        .output_details
        .iter()
        .find(|o| o.role == OutputRole::Received)
        .and_then(|o| Some((o.address.as_ref()?.to_string(), o.value)));
    let (mut kind, chain) = kind_of(record.direction, record.transaction_type, sent_to.clone());
    // Paid out: the payee's address; received — from someone else or from one of our own
    // addresses (BUG-33) — our own address it came to: "Receive" lists them.
    let address = match &mut kind {
        ActivityKind::SelfTransfer { moved } => received.map(|(address, value)| {
            *moved = value as Duffs;
            address
        }),
        _ if record.direction == TransactionDirection::Incoming => {
            received.map(|(address, _)| address)
        }
        _ => sent_to,
    };
    // In a block: the block's time; in the mempool: when we saw it.
    let at = record
        .context
        .block_info()
        .map_or(seen, |b| b.timestamp() as Timestamp);
    let txid = record.txid.to_string();
    let (layers, flow) = crate::layers::of(&kind, chain);
    let proof = proof(&record.context, instant_verified);
    let block = record.context.block_info().map(|b| b.height());
    let detail = detail_of(record, &kind, proof, block);
    Activity {
        key: format!("core:{txid}"),
        id: u64::from_str_radix(&txid[..16.min(txid.len())], 16).unwrap_or(0),
        kind,
        chain,
        at,
        amount: net,
        proof,
        txid: Some(txid),
        // Only what this wallet paid: an incoming payment's fee is the sender's. An asset lock is
        // always this wallet's, whatever its net says (see `Real::wallet` on locks for the pool).
        fee: (net < 0 || record.transaction_type == TransactionType::AssetLock)
            .then_some(record.fee)
            .flatten()
            .map(|f| f as Duffs),
        block,
        address,
        layers,
        flow,
        detail: Some(Box::new(detail)),
        note: None,
        rate: None,
        tax_transfer: false,
    }
}

/// The addresses of the wallet's CoinJoin accounts, which [`mark_mixing`] tells mixing lines by.
pub fn coinjoin_addresses(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> std::collections::HashSet<Address> {
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    info.accounts()
        .coinjoin_accounts
        .values()
        .flat_map(|a| a.keys().managed_account_type().all_addresses())
        .collect()
}

/// A Core line of the history as FEAT-0030 A3 shows it, from the record's own details
/// (`coinjoin`: the wallet's CoinJoin addresses, [`coinjoin_addresses`]):
/// - a mixing round ([`evolite_coinjoin::mixing_shape`]; key-wallet's own mark needs 3
///   participants) — «Mixing (CoinJoin)», its fee what left the wallet (none as a rule);
/// - a collateral a session took from a CoinJoin address — the same, its fee what was taken;
/// - the preparation: ordinary coins of ours into denominations on CoinJoin addresses, nothing to
///   anyone else — «Into mixing»;
/// - a payment from CoinJoin coins — «Sent» as it was, in the «Mixed» layer instead of Core.
///
/// Anything else keeps what [`activity`] made of it.
pub fn mark_mixing(
    act: &mut Activity,
    record: &TransactionRecord,
    coinjoin: &std::collections::HashSet<Address>,
) {
    use evolite_api::layer::MIXED;
    use evolite_coinjoin::{is_collateral, is_denominated};
    if coinjoin.is_empty() || record.transaction_type != TransactionType::Standard {
        return;
    }
    let on_coinjoin = |a: Option<&Address>| a.is_some_and(|a| coinjoin.contains(a));
    let ours_in: Vec<u64> = record.input_details.iter().map(|d| d.value).collect();
    let owned: Vec<_> = record
        .output_details
        .iter()
        .filter(|o| matches!(o.role, OutputRole::Received | OutputRole::Change))
        .collect();
    let ours_out: u64 = owned.iter().map(|o| o.value).sum();
    let paid_out = record
        .output_details
        .iter()
        .any(|o| o.role == OutputRole::Sent);
    let any_coinjoin_in = record
        .input_details
        .iter()
        .any(|d| coinjoin.contains(&d.address));
    let from_coinjoin = !record.input_details.is_empty()
        && record
            .input_details
            .iter()
            .all(|d| coinjoin.contains(&d.address));
    let spent: u64 = ours_in.iter().sum();
    let kind = if evolite_coinjoin::mixing_shape(&record.transaction, &ours_in, ours_out) {
        Some(ActivityKind::Mixing { fee: 0 })
    } else if from_coinjoin
        && ours_in.iter().all(|v| is_collateral(*v))
        && owned.iter().all(|o| is_collateral(o.value))
        && spent > ours_out
    {
        Some(ActivityKind::Mixing {
            fee: (spent - ours_out) as Duffs,
        })
    } else if !paid_out
        && !record.input_details.is_empty()
        && !any_coinjoin_in
        && owned
            .iter()
            .any(|o| on_coinjoin(o.address.as_ref()) && is_denominated(o.value))
        && owned.iter().all(|o| {
            !on_coinjoin(o.address.as_ref()) || is_denominated(o.value) || is_collateral(o.value)
        })
    {
        Some(ActivityKind::IntoMixing)
    } else {
        None
    };
    match kind {
        Some(kind) => {
            (act.layers, act.flow) = crate::layers::of(&kind, act.chain);
            if let ActivityKind::Mixing { fee } = kind {
                act.fee = (fee > 0).then_some(fee);
            }
            act.address = None;
            act.kind = kind;
        }
        None if from_coinjoin && matches!(act.kind, ActivityKind::Sent { .. }) => {
            act.layers = MIXED;
        }
        None => {}
    }
}

/// What proved a Core operation, in the card's words (FEAT-0005 AC-T3): an InstantLock only as
/// [`proof`] gives it — verified (BUG-26); a ChainLock with its block. Nothing for the mempool or
/// a block not locked yet: the chip says that.
pub fn confirm(proof: Proof, block: Option<u32>) -> Option<Confirm> {
    match proof {
        Proof::InstantLock => Some(Confirm::InstantLock),
        Proof::ChainLock => block.map(|block| Confirm::ChainLock { block }),
        _ => None,
    }
}

/// R-5 (the architect, answer 3): a payment of this wallet that can be broadcast again — a
/// standard transaction going out or between own addresses, not in a block yet. Not an asset
/// lock or unlock, not a mixing round, not one coming in, not one confirmed (batch 63: the card
/// would offer it).
pub fn resendable(record: &TransactionRecord) -> bool {
    record.transaction_type == TransactionType::Standard
        && matches!(
            record.direction,
            TransactionDirection::Outgoing | TransactionDirection::Internal
        )
        && !confirmed(record)
}

/// In a block, ChainLocked or not.
pub fn confirmed(record: &TransactionRecord) -> bool {
    matches!(
        record.context,
        TransactionContext::InBlock(_) | TransactionContext::InChainLockedBlock(_)
    )
}

/// The card's own addresses of a Core transaction (FEAT-0005 АРХ-3): ours it spent, one per
/// input, and ours that got coins — `Received` only, change is not shown (designer-16). From the
/// record's details, which [`one_per_transaction`] and [`attribute_late_inputs`] made whole over
/// the accounts. Incomplete where the kind needs what the record lacks: a transfer between own
/// addresses with no input or no receiving output of ours (one account's view of a transfer
/// between accounts), a payment out with no input of ours, a payment in with no output of ours.
fn detail_of(
    record: &TransactionRecord,
    kind: &ActivityKind,
    proof: Proof,
    block: Option<u32>,
) -> Detail {
    let own_from: Vec<String> = record
        .input_details
        .iter()
        .map(|d| d.address.to_string())
        .collect();
    let own_to: Vec<(String, Duffs)> = record
        .output_details
        .iter()
        .filter(|o| o.role == OutputRole::Received)
        .filter_map(|o| Some((o.address.as_ref()?.to_string(), o.value as Duffs)))
        .collect();
    let incomplete = match kind {
        ActivityKind::SelfTransfer { .. } => own_from.is_empty() || own_to.is_empty(),
        ActivityKind::Sent { .. } => own_from.is_empty(),
        ActivityKind::Received { .. } => own_to.is_empty(),
        _ => false,
    };
    Detail {
        confirm: confirm(proof, block),
        own_from,
        own_to,
        incomplete,
        resendable: resendable(record),
        ..Detail::default()
    }
}

/// A spend the wallet saw before the coin it spends. A restore finds a coin on an address
/// beyond the gap limit only after a rescan, and then the spending transaction is already
/// recorded — from its change alone, as "Received" the change. key-wallet attributes
/// such late inputs only to records it still keeps in memory; a chain-locked one is dropped by
/// then, and the store keeps it uncorrected. Here every record's inputs are matched against
/// the wallet's own outputs in the other records; a record that spends one of them gets the
/// input, the outputs it did not list — by `is_ours`, the same rule as [`own_outputs`]: ours
/// (`Received`, `Change`) or paid out (`Sent`) — and its net, direction and fee again (the
/// same rules as key-wallet's). BUG-232 (QA batch 31, qa-f31): a payment between own
/// identities kept in the store as the contact account's slice lists only the contact's
/// output; its change, unlisted, was taken as paid out («Отправлено ydp4… −0,1452581»).
pub fn attribute_late_inputs(
    records: &mut [TransactionRecord],
    is_ours: impl Fn(&Address) -> Option<OutputRole>,
) {
    use key_wallet::dashcore::OutPoint;
    use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};
    use std::collections::HashMap;

    // The outputs of ours the inputs are matched against: those a record lists as ours, and
    // (BUG-232, QA batch 36: qa-f31's tx2 spends tx1's change, unlisted in tx1's stored slice)
    // those it does not list that `is_ours` knows — the same rule as for the outputs below.
    let mut ours: HashMap<OutPoint, (u64, Address)> = HashMap::new();
    for r in records.iter() {
        for (index, output) in r.transaction.output.iter().enumerate() {
            let index = index as u32;
            let listed = r.output_details.iter().find(|d| d.index == index);
            // Listed as ours, or else by the same rule as the outputs (`is_ours`, by address):
            // unlisted, or listed as paid out by an account that does not know the address.
            // A contact's watch account lists the contact's coins as received (BUG-239): its
            // roles are not the wallet's; only `is_ours` says.
            let address = match listed {
                Some(d)
                    if !is_contact_watch(r)
                        && matches!(d.role, OutputRole::Received | OutputRole::Change) =>
                {
                    d.address.clone()
                }
                Some(d) if d.role == OutputRole::Unspendable => None,
                _ => Address::from_script(&output.script_pubkey, crate::network::dash())
                    .ok()
                    .filter(|a| is_ours(a).is_some()),
            };
            if let Some(address) = address {
                ours.insert(OutPoint::new(r.txid, index), (output.value, address));
            }
        }
    }
    for r in records.iter_mut() {
        let mut added = false;
        for (index, input) in r.transaction.input.iter().enumerate() {
            let index = index as u32;
            if r.input_details.iter().any(|d| d.index == index) {
                continue;
            }
            if let Some((value, address)) = ours.get(&input.previous_output) {
                r.input_details.push(InputDetail {
                    index,
                    value: *value,
                    address: address.clone(),
                });
                added = true;
            }
        }
        if !added {
            continue;
        }
        r.input_details.sort_by_key(|d| d.index);
        // What the record did not list: ours by the same rule as `own_outputs`, else paid to
        // someone, or a data output.
        for (index, output) in r.transaction.output.iter().enumerate() {
            let index = index as u32;
            if r.output_details.iter().any(|d| d.index == index) {
                continue;
            }
            let address = Address::from_script(&output.script_pubkey, crate::network::dash()).ok();
            let role = if output.script_pubkey.is_provably_unspendable() {
                OutputRole::Unspendable
            } else {
                address
                    .as_ref()
                    .and_then(&is_ours)
                    .unwrap_or(OutputRole::Sent)
            };
            r.output_details.push(OutputDetail {
                index,
                role,
                address,
                value: output.value,
            });
        }
        r.output_details.sort_by_key(|d| d.index);
        let owned: i64 = r
            .output_details
            .iter()
            .filter(|o| matches!(o.role, OutputRole::Received | OutputRole::Change))
            .map(|o| o.value as i64)
            .sum();
        let spent: i64 = r.input_details.iter().map(|i| i.value as i64).sum();
        r.net_amount = owned - spent;
        let sent = r.output_details.iter().any(|d| d.role == OutputRole::Sent);
        r.direction = if r.transaction_type == TransactionType::CoinJoin {
            TransactionDirection::CoinJoin
        } else if !sent && owned > 0 {
            TransactionDirection::Internal
        } else {
            TransactionDirection::Outgoing
        };
        // Every input ours: the fee is what they held over what the outputs carry.
        if r.input_details.len() == r.transaction.input.len() {
            let out: u64 = r.transaction.output.iter().map(|o| o.value).sum();
            r.fee = (spent as u64).checked_sub(out);
        }
    }
}

/// Core payments pay the network's usual rate, 1 duff a byte.
pub fn fee_rate() -> FeeRate {
    FeeRate::normal()
}

/// "Fee, at most" of a transfer of credits. Platform meters these writes and nothing gives their
/// cost in advance; its protocol minimum (100 duffs) is no bound. Seen on testnet: to an identity
/// 2 901 duffs (manual pass 16), to a Platform address about 3 400 (live run of 2026-11-08) and
/// 9 900 (the skill `platform-wallet`). These are our margin over that, not a limit of the protocol.
const CREDIT_TRANSFER: Duffs = 5_000;
const CREDIT_TRANSFER_TO_ADDRESS: Duffs = 20_000;
/// How many of the last transfers "at most" looks at: enough that a cheap one after a dear one does
/// not bring it down, few enough that one fee misread from a stale balance does not stay for good.
const RECENT_TRANSFERS: usize = 8;

/// How many of the last metered writes of a kind its estimate looks at (BUG-197).
const RECENT_WRITES: usize = 5;

/// What the network wants on the identity before it writes a contact request (BUG-107): Platform
/// compares the balance with an estimate (`validate_fees_of_event`, a dry run at worst-case tree
/// costs), not with the fee, and then takes less. Manual pass 16 was refused at 126 042 duffs and
/// accepted at 226 042; where between is not known, this is above the accepted one.
const CONTACT_REQUEST_BALANCE: Duffs = 230_000;

/// A contact request on testnet, 2026-11-05: 0.00102028 DASH (manual run M-7), rounded up.
const CONTACT_REQUEST: Duffs = 105_000;
/// A DashPay profile's first write on testnet, 2026-11-15: 0.0004685 DASH (QA pass 24, C4), with
/// about a third on top (BUG-171 P3). It was priced as a contact request, 0.00105: more than twice.
const PROFILE: Duffs = 62_000;
/// A regular DPNS name on testnet, 2026-11-05: 0.00057979 DASH (evolite-test-data), rounded up.
/// What a *later* name costs: a first name on an identity measured about 77 200 and the form
/// shows [`Fees::name_price`] (BUG-97). What it needs up front is not this: see
/// [`Fees::name_credits_needed`].
const NAME_REGULAR: Duffs = 65_000;
/// What goes on top for an identity with its six keys (BUG-93). The firm number is the live
/// 222 937 duffs of a creation and a first name together (2026-11-08); the creation alone is
/// inferred, and is between about 147 000 (Platform's fixtures per key) and 165 000 (the total
/// less a name's 57 979). Platform's processing fee depends on the state of its trees, so this
/// has a margin. Not the protocol's minimum, which is only the floor of what a lock must hold
/// (`identity_lock_min`).
const IDENTITY_CREATE: Duffs = 175_000;

/// What an identity's asset lock holds and takes from Core (`create_identity`): the amount
/// asked for and the fees on top of it, with a margin (BUG-93), so the amount reaches the
/// identity whole and what the margin leaves stays as credits — or `None` below
/// [`Fees::identity_amount_least`]: the lock would be refused, or the name short of credits
/// with the DASH already locked. Without a name (`named` false, FEAT-0014 R-7) nothing is paid
/// for one.
pub fn identity_lock(top_up: u64, fees: &Fees, named: bool) -> Option<u64> {
    let amount = Duffs::try_from(top_up).unwrap_or(Duffs::MAX);
    (amount >= fees.identity_amount_least(named))
        .then(|| fees.identity_lock_for(amount, named) as u64)
}

/// What a top-up's asset lock holds (`top_up_identity`): the amount asked for and Platform's fee
/// on top of it (BUG-93), so the amount reaches the identity whole — or `None` below
/// [`Fees::top_up_amount_least`]: Platform would refuse the lock, its DASH would stay in it and
/// every next top-up would wait for it (review must 1).
pub fn top_up_lock(amount: u64, fees: &Fees) -> Option<u64> {
    let enough = Duffs::try_from(amount).unwrap_or(Duffs::MAX) >= fees.top_up_amount_least();
    enough.then(|| amount.saturating_add(fees.top_up_on_top() as u64))
}

/// The least a top-up's asset lock may hold, in duffs, at protocol `version`: dpp's
/// `calculate_min_required_fee_v1` of an identity top-up — the base cost and the asset lock's
/// processing start.
pub fn top_up_lock_min(version: &dpp::version::PlatformVersion) -> Duffs {
    let min = &version.fee_version.state_transition_min_fees;
    let start = version
        .dpp
        .state_transitions
        .identities
        .asset_locks
        .required_asset_lock_duff_balance_for_processing_start_for_identity_top_up;
    (min.identity_topup_base_cost.div_ceil(1000) + start) as Duffs
}

/// The least an asset lock may hold to fund a Platform address, in duffs, at protocol
/// `version` (FEAT-0015 stage 2): dpp's own `calculate_min_required_fee` of the transition with
/// no address inputs and one output — the asset lock's processing start and the output's cost.
/// A smaller lock is refused and its DASH stay locked.
pub fn address_fund_lock_min(version: &dpp::version::PlatformVersion) -> Duffs {
    use dpp::state_transition::address_funding_from_asset_lock_transition::calculate_address_funding_from_asset_lock_min_required_fee as min_fee;
    min_fee(0, 1, version).div_ceil(1000) as Duffs
}

/// The least an identity's asset lock may hold, in duffs, at protocol `version`: dpp's
/// `calculate_min_required_fee_v1` of an identity create (protocol 3 on) with `keys` keys — the
/// base cost, the asset lock's processing start and each key's.
pub fn identity_lock_min(version: &dpp::version::PlatformVersion, keys: u64) -> Duffs {
    let min = &version.fee_version.state_transition_min_fees;
    let start = version
        .dpp
        .state_transitions
        .identities
        .asset_locks
        .required_asset_lock_duff_balance_for_processing_start_for_identity_create;
    let credits = min
        .identity_create_base_cost
        .saturating_add(min.identity_key_in_creation_cost.saturating_mul(keys));
    (credits.div_ceil(1000) + start) as Duffs
}

/// The wallet's unspent Core coins, each once (BUG-77, R-34): the coins of every funds-bearing
/// account but the CoinJoin ones, one per outpoint. platform-wallet's balance adds the accounts
/// up, and the store's early load (BUG-65) puts a contact's DIP-15 coin whose owner it does not
/// find into account 0 («funds-account-fallback»); the filters then find it again in its own
/// DashPay account, and the sum counted it twice (werner555: 3,9172 instead of 3,8657 DASH).
///
/// The CoinJoin account's coins are not Core coins here (FEAT-0030 AC-T2, BUG-121): a payment
/// never takes them (spending them with others undoes the mixing), so the Core balance, its
/// checks and its coin steps leave them out too; [`mixed_coins`] has them.
pub fn coins_once(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> Vec<&key_wallet::Utxo> {
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let accounts = info.accounts();
    let mut seen = std::collections::HashSet::new();
    accounts
        .standard_bip44_accounts
        .values()
        .chain(accounts.standard_bip32_accounts.values())
        .chain(accounts.dashpay_receival_accounts.values())
        .flat_map(|account| account.utxos.values())
        .filter(|utxo| seen.insert(utxo.outpoint))
        .collect()
}

/// A coin of the CoinJoin account, with how far it is mixed (FEAT-0030 AC-T3).
#[derive(Debug, Clone)]
pub struct MixedCoin {
    pub utxo: key_wallet::Utxo,
    pub rounds: evolite_coinjoin::Rounds,
    /// A denomination mixed at least [`evolite_coinjoin::FULLY_MIXED_ROUNDS`] times, and final
    /// ([`coin_is_final`]): what «Mixed» may spend.
    pub spendable: bool,
}

/// The wallet's own transactions and scripts, as evolite-coinjoin counts rounds over them.
struct OwnHistory<'a> {
    txs: std::collections::HashMap<
        key_wallet::dashcore::Txid,
        &'a key_wallet::dashcore::Transaction,
    >,
    ours: std::collections::HashSet<key_wallet::dashcore::ScriptBuf>,
}

impl evolite_coinjoin::History for OwnHistory<'_> {
    fn transaction(
        &self,
        txid: &key_wallet::dashcore::Txid,
    ) -> Option<&key_wallet::dashcore::Transaction> {
        self.txs.get(txid).copied()
    }
    fn is_ours(&self, script: &key_wallet::dashcore::Script) -> bool {
        self.ours.contains(script)
    }
}

fn own_history(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> OwnHistory<'_> {
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let accounts = info.accounts().all_funding_accounts();
    OwnHistory {
        txs: accounts
            .iter()
            .flat_map(|a| a.transactions().values())
            .map(|r| (r.transaction.txid(), &r.transaction))
            .collect(),
        ours: accounts
            .iter()
            .flat_map(|a| a.keys().managed_account_type().all_addresses())
            .map(|address| address.script_pubkey())
            .collect(),
    }
}

/// The coins of the wallet's CoinJoin accounts, each once, with their rounds counted in one pass
/// over the wallet's history (FEAT-0030 AC-T3, AC-T8): spendable when fully mixed and final
/// (`verified`: the InstantLocks checked in this run, as [`coin_is_final`]).
pub fn mixed_coins(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    verified: impl Fn(&key_wallet::dashcore::Txid) -> bool,
) -> Vec<MixedCoin> {
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let mut seen = std::collections::HashSet::new();
    let coins: Vec<&key_wallet::Utxo> = info
        .accounts()
        .coinjoin_accounts
        .values()
        .flat_map(|account| account.utxos.values())
        .filter(|utxo| seen.insert(utxo.outpoint))
        .collect();
    if coins.is_empty() {
        return Vec::new();
    }
    let history = own_history(info);
    let mut counter = evolite_coinjoin::RoundCounter::new(&history);
    coins
        .into_iter()
        .map(|utxo| {
            let rounds = counter.rounds(&utxo.outpoint);
            MixedCoin {
                utxo: utxo.clone(),
                rounds,
                spendable: rounds.fully_mixed()
                    && evolite_coinjoin::is_denominated(utxo.value())
                    && coin_is_final(utxo, &verified),
            }
        })
        .collect()
}

/// The coins of the CoinJoin accounts that have not been through all the rounds, with how many
/// they have (FEAT-0030 AC-A6): «Coins» lists them apart, and a payment takes them only alone.
pub fn unfinished_coins(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    verified: impl Fn(&key_wallet::dashcore::Txid) -> bool,
) -> Vec<(key_wallet::Utxo, u8)> {
    mixed_coins(info, verified)
        .into_iter()
        .filter(|c| !c.rounds.fully_mixed())
        .map(|c| {
            let rounds = match c.rounds {
                evolite_coinjoin::Rounds::Mixed(n) => n,
                _ => 0,
            };
            (c.utxo, rounds)
        })
        .collect()
}

/// «Mixed» on the overview (FEAT-0030 AC-T3): `None` when the CoinJoin accounts hold nothing.
pub fn mixed_balance(coins: &[MixedCoin]) -> Option<Mixed> {
    if coins.is_empty() {
        return None;
    }
    let count = coins.iter().filter(|c| c.spendable).count();
    let (spendable, unfinished) = coins.iter().fold((0, 0), |(s, u), c| {
        let value = c.utxo.value() as Duffs;
        if c.spendable {
            (s + value, u)
        } else {
            (s, u + value)
        }
    });
    Some(Mixed {
        spendable,
        unfinished,
        rounds_needed: evolite_coinjoin::FULLY_MIXED_ROUNDS,
        max: if count == 0 {
            0
        } else {
            (spendable - crate::mixed_pay::usual_fee(count) as Duffs).max(0)
        },
    })
}

/// Whether a coin can be spent as final: in a block, or InstantSend-locked by a lock whose signature
/// was verified in this run (`verified`, R-1). dash-spv marks a coin locked before its lock is
/// checked, and on any lock at all (BUG-111), so key-wallet's `is_instantlocked` alone says nothing.
pub fn coin_is_final(
    coin: &key_wallet::Utxo,
    verified: impl Fn(&key_wallet::dashcore::Txid) -> bool,
) -> bool {
    coin.is_confirmed || (coin.is_instantlocked && verified(&coin.outpoint.txid))
}

/// TECH-finality-journal: of the coins of `info`'s funding accounts, how many are in a block,
/// InstantSend-locked by a lock verified in this run (`verified`), and locked by one that was not.
pub fn finality_counts(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    verified: impl Fn(&key_wallet::dashcore::Txid) -> bool,
) -> (u32, u32, u32) {
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let (mut confirmed, mut is_verified, mut is_unverified) = (0, 0, 0);
    for account in info.accounts().all_funding_accounts() {
        for utxo in account.utxos.values() {
            if utxo.is_confirmed {
                confirmed += 1;
            } else if utxo.is_instantlocked && verified(&utxo.outpoint.txid) {
                is_verified += 1;
            } else if utxo.is_instantlocked {
                is_unverified += 1;
            }
        }
    }
    (confirmed, is_verified, is_unverified)
}

/// SPV item 3: locks (key-wallet's `Utxo::lock`) each coin of `info` that is not final
/// ([`coin_is_final`] by `verified`), so that platform-wallet's own builders (the asset lock, a
/// payment to a contact) do not take it: they choose among `is_spendable` coins, and an
/// InstantSend mark that dash-spv set on a lock it did not check makes a coin look final to them.
/// `keep_trusted`: our own change waiting for its block stays (a payment to a contact takes it,
/// as a Send does, R-5). `held` counts the builds under way that froze each coin; a coin locked
/// by someone else (a payment's reservation) is left alone. Answers the coins this build holds,
/// for [`thaw`]. Locks are not stored: a restart frees them all.
pub fn freeze_unfinal(
    info: &mut key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    verified: impl Fn(&key_wallet::dashcore::Txid) -> bool,
    keep_trusted: bool,
    held: &mut std::collections::HashMap<key_wallet::dashcore::OutPoint, u32>,
) -> Vec<key_wallet::dashcore::OutPoint> {
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let mut frozen = std::collections::BTreeSet::new();
    for account in info.accounts_mut().all_funding_accounts_mut() {
        for utxo in account.utxos.values_mut() {
            if coin_is_final(utxo, &verified) || (keep_trusted && utxo.is_trusted) {
                continue;
            }
            // Ours already (this build's copy in another account, or another build's): held.
            if held.contains_key(&utxo.outpoint) || frozen.contains(&utxo.outpoint) {
                utxo.lock();
                frozen.insert(utxo.outpoint);
                continue;
            }
            if utxo.is_locked {
                continue;
            }
            utxo.lock();
            frozen.insert(utxo.outpoint);
        }
    }
    for outpoint in &frozen {
        *held.entry(*outpoint).or_insert(0) += 1;
    }
    info.update_balance();
    frozen.into_iter().collect()
}

/// Undoes [`freeze_unfinal`] for the coins one build held: each is unlocked once no build holds
/// it any more (AC-T3.4: none frozen for good, whatever the build's outcome).
pub fn thaw(
    info: &mut key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    frozen: &[key_wallet::dashcore::OutPoint],
    held: &mut std::collections::HashMap<key_wallet::dashcore::OutPoint, u32>,
) {
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let mut free = std::collections::HashSet::new();
    for outpoint in frozen {
        match held.get_mut(outpoint) {
            Some(n) if *n > 1 => *n -= 1,
            Some(_) => {
                held.remove(outpoint);
                free.insert(*outpoint);
            }
            None => {}
        }
    }
    if free.is_empty() {
        return;
    }
    for account in info.accounts_mut().all_funding_accounts_mut() {
        for utxo in account.utxos.values_mut() {
            if free.contains(&utxo.outpoint) {
                utxo.unlock();
            }
        }
    }
    info.update_balance();
}

/// The coins a payment may take: of the accounts of `SEND_FUNDING_SOURCES` at index 0, each once,
/// and final ([`coin_is_final`]). Coins of another account could not be signed for. Nor a
/// coin its account does not derive the address of (BUG-142, [`unsignable`]): a copy another
/// account holds as its own is taken instead.
pub fn payable_coins(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    verified: impl Fn(&key_wallet::dashcore::Txid) -> bool,
) -> Vec<key_wallet::Utxo> {
    use key_wallet::account::ManagedAccountTrait;
    let mut seen = std::collections::HashSet::new();
    send_accounts(info)
        .flat_map(|account| {
            account
                .utxos
                .values()
                .filter(move |u| account.address_derivation_path(&u.address).is_some())
        })
        .filter(|utxo| coin_is_final(utxo, &verified) && seen.insert(utxo.outpoint))
        .cloned()
        .collect()
}

/// What each address of the live wallet's coin-bearing accounts is to it (BUG-194): change (an
/// internal pool) or received, by script — the accounts `own_scripts` reads from the store: BIP44,
/// BIP32, CoinJoin and DashPay receiving. platform-wallet writes a pool to the store only when it
/// registers an account, so the store's table ends at the first gap window (index 0–29): change
/// past it was not ours to the history, a payment of 0.0001 showed as its change paid out. The
/// live wallet derives its pools past every coin it restores and the gap after (platform-wallet
/// `rehydrate`), so it knows them.
pub fn live_own(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> std::collections::HashMap<key_wallet::dashcore::ScriptBuf, OutputRole> {
    use key_wallet::managed_account::address_pool::AddressPoolType;
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let a = info.accounts();
    let accounts = a
        .standard_bip44_accounts
        .values()
        .chain(a.standard_bip32_accounts.values())
        .chain(a.coinjoin_accounts.values())
        .chain(a.dashpay_receival_accounts.values());
    let mut own = std::collections::HashMap::new();
    for account in accounts {
        for pool in account.keys().managed_account_type().address_pools() {
            let role = if pool.pool_type == AddressPoolType::Internal {
                OutputRole::Change
            } else {
                OutputRole::Received
            };
            for script in pool.script_pubkey_index.keys() {
                own.entry(script.clone()).or_insert(role);
            }
        }
    }
    own
}

/// BUG-232: how far past the last address it has made a pool of ours is searched for a
/// payment to it. A pool grows only from the addresses it has seen used (the gap limit, 30), and
/// key-wallet reads each transaction once: on a fresh copy of an older data folder, a payment
/// to an address beyond the pool as it then stood stays «sent», though the address is ours.
pub const GAP_LOOKAHEAD: u32 = 100;

/// The outputs of `records` not known as ours (role `Sent`), each with the block height of its
/// record (0 out of a block): what [`adopt_addresses_past_the_gap`] looks for.
pub fn paid_outside(records: &[TransactionRecord]) -> Vec<(Address, u32)> {
    records
        .iter()
        .flat_map(|r| {
            let height = r.context.block_info().map_or(0, |b| b.height());
            r.output_details
                .iter()
                .filter(|d| d.role == OutputRole::Sent)
                .filter_map(move |d| d.address.clone().map(|a| (a, height)))
        })
        .collect()
}

/// A pool of ours that takes in addresses past its gap ([`find_past_the_gap`]): the pool as it
/// will be, and what it takes in.
#[derive(Debug)]
pub struct PastTheGap {
    bip44: bool,
    account: u32,
    pool_type: key_wallet::managed_account::address_pool::AddressPoolType,
    /// The pool's highest used and made index when it was read: put in place only if it has
    /// not moved since (a block may have grown it meanwhile; it is looked at again then).
    was: (Option<u32>, Option<u32>),
    pool: key_wallet::managed_account::address_pool::AddressPool,
    adopted: Vec<Address>,
}

/// BUG-232 (qa-f29, tx1): which of the BIP44 and BIP32 pools would take in addresses of `paid`
/// that are theirs within [`GAP_LOOKAHEAD`] past the last address each has made, or made already
/// and not marked used (review r1 S1), and how each
/// would be then: made up to the furthest one found, those marked used, and the usual gap after
/// them — as key-wallet does for an address it sees used; then again from there, so a run of
/// them is found. Only addresses derived from our own keys can match. Read-only, on copies of
/// the pools: the derivations happen outside the wallet's write lock.
pub fn find_past_the_gap(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    keys: &key_wallet::Wallet,
    paid: &[Address],
) -> Vec<PastTheGap> {
    use key_wallet::managed_account::address_pool::KeySource;
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    let mut finds = Vec::new();
    if paid.is_empty() {
        return finds;
    }
    let accounts = keys
        .accounts
        .standard_bip44_accounts
        .iter()
        .map(|(n, a)| (true, *n, a.account_xpub))
        .chain(
            keys.accounts
                .standard_bip32_accounts
                .iter()
                .map(|(n, a)| (false, *n, a.account_xpub)),
        );
    for (bip44, n, xpub) in accounts {
        let managed = if bip44 {
            info.accounts.standard_bip44_accounts.get(&n)
        } else {
            info.accounts.standard_bip32_accounts.get(&n)
        };
        let Some(account) = managed else {
            continue;
        };
        let source = KeySource::Public(xpub);
        for original in account.keys().managed_account_type().address_pools() {
            let mut pool = original.clone();
            let mut adopted: Vec<Address> = Vec::new();
            // A run of them: each one taken in moves the pool's end; a few rounds at most.
            for _ in 0..8 {
                let made = pool.highest_generated.map_or(0, |h| h + 1);
                let used = pool.highest_used.map_or(0, |h| h + 1);
                // `maintain_gap_limit` makes up to the highest used + the gap.
                let mut probe = pool.clone();
                probe.gap_limit = (made + GAP_LOOKAHEAD).saturating_sub(used).max(1);
                if probe.maintain_gap_limit(&source).is_err() {
                    break;
                }
                // Past the end, or (review r1 S1: a wallet restarted since, whose pool came
                // from the store) made already but not marked used.
                let found: Vec<(u32, Address)> = paid
                    .iter()
                    .filter(|a| !adopted.contains(a))
                    .filter_map(|a| {
                        let i = *probe.address_index.get(a)?;
                        (i >= made || !pool.used_indices.contains(&i)).then(|| (i, a.clone()))
                    })
                    .collect();
                let Some(top) = found.iter().map(|(i, _)| *i).max() else {
                    break;
                };
                // The pool itself, not the probe: made to the furthest found, those used, then
                // its own gap after them (the probe's other addresses are not kept).
                let gap = pool.gap_limit;
                pool.gap_limit = (top + 1).saturating_sub(used).max(1);
                if pool.maintain_gap_limit(&source).is_err() {
                    break;
                }
                for (_, address) in &found {
                    pool.mark_used(address);
                }
                pool.gap_limit = gap;
                if pool.maintain_gap_limit(&source).is_err() {
                    break;
                }
                adopted.extend(found.into_iter().map(|(_, a)| a));
            }
            if !adopted.is_empty() {
                finds.push(PastTheGap {
                    bip44,
                    account: n,
                    pool_type: original.pool_type,
                    was: (original.highest_used, original.highest_generated),
                    pool,
                    adopted,
                });
            }
        }
    }
    finds
}

/// Puts the pools [`find_past_the_gap`] made in place, each only if it has not moved since it
/// was read, and tells the account its watched addresses changed (`bump_monitor_revision`, as
/// key-wallet after its own gap upkeep). Answers the addresses taken in.
pub fn take_in_past_the_gap(
    info: &mut key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    finds: Vec<PastTheGap>,
) -> Vec<Address> {
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    let mut adopted = Vec::new();
    for find in finds {
        let managed = if find.bip44 {
            info.accounts.standard_bip44_accounts.get_mut(&find.account)
        } else {
            info.accounts.standard_bip32_accounts.get_mut(&find.account)
        };
        let Some(account) = managed else {
            continue;
        };
        let mut put = false;
        for pool in account
            .keys_mut()
            .managed_account_type_mut()
            .address_pools_mut()
        {
            if pool.pool_type == find.pool_type
                && (pool.highest_used, pool.highest_generated) == find.was
            {
                *pool = find.pool.clone();
                put = true;
            }
        }
        if put {
            account.bump_monitor_revision();
            adopted.extend(find.adopted);
        }
    }
    adopted
}

/// BUG-232 review r1 M1: the live wallet's coins of outputs that `records` (and the live
/// wallet's own) read as paid out (role `Sent`): those key-wallet credited when a rescan read a
/// chain-locked record again after its address was taken in (vendor
/// `evolite-finalized-outputs.patch`). platform-wallet writes a coin to the store only from a new
/// record's `Received`/`Change` outputs, so these are written by
/// [`crate::chain::Chain::save_coins`].
pub fn coins_of_sent_outputs(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    records: &[TransactionRecord],
) -> Vec<key_wallet::Utxo> {
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let accounts = info.accounts();
    let live: Vec<TransactionRecord> = accounts
        .all_accounts()
        .into_iter()
        .flat_map(|a| a.transactions().values().cloned())
        .collect();
    let sent: std::collections::HashSet<key_wallet::dashcore::OutPoint> = records
        .iter()
        .chain(live.iter())
        .flat_map(|r| {
            r.output_details
                .iter()
                .filter(|d| d.role == OutputRole::Sent)
                .map(move |d| key_wallet::dashcore::OutPoint::new(r.txid, d.index))
        })
        .collect();
    let mut coins: Vec<key_wallet::Utxo> = accounts
        .all_funding_accounts()
        .into_iter()
        .flat_map(|a| a.utxos.values())
        .filter(|u| sent.contains(&u.outpoint))
        .cloned()
        .collect();
    coins.sort_by_key(|u| u.outpoint);
    coins.dedup_by_key(|u| u.outpoint);
    coins
}

/// BUG-232 review r1 (M1, S1, and a close before the rescan): outputs read as paid out (role
/// `Sent`) of records in a block that pay an address of our funding accounts' pools now, and are
/// neither a coin of ours, nor spent by a record we have (`records`, and the live wallet's own),
/// nor seen spent by the wallet: each with its block height. A rescan of that block gives them
/// their coin (vendor `evolite-finalized-outputs.patch`); until then, after a close too, they
/// are found here again.
pub fn uncredited_outputs(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    records: &[TransactionRecord],
) -> Vec<(key_wallet::dashcore::OutPoint, u32)> {
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let accounts = info.accounts();
    let live: Vec<&TransactionRecord> = accounts
        .all_accounts()
        .into_iter()
        .flat_map(|a| a.transactions().values())
        .collect();
    let all = || records.iter().chain(live.iter().copied());
    let spent: std::collections::HashSet<key_wallet::dashcore::OutPoint> = all()
        .flat_map(|r| r.transaction.input.iter().map(|i| i.previous_output))
        .chain(info.observed_spent_outpoints().keys().copied())
        .collect();
    let funding = accounts.all_funding_accounts();
    let ours = |a: &Address| {
        funding.iter().any(|f| {
            f.keys()
                .managed_account_type()
                .address_pools()
                .iter()
                .any(|p| p.contains_address(a))
        })
    };
    let mut found: Vec<(key_wallet::dashcore::OutPoint, u32)> = Vec::new();
    for r in all() {
        let Some(height) = r.context.block_info().map(|b| b.height()) else {
            continue;
        };
        for d in r
            .output_details
            .iter()
            .filter(|d| d.role == OutputRole::Sent)
        {
            let outpoint = key_wallet::dashcore::OutPoint::new(r.txid, d.index);
            if d.address.as_ref().is_some_and(ours)
                && !spent.contains(&outpoint)
                && !funding.iter().any(|f| f.utxos.contains_key(&outpoint))
                && !found.iter().any(|(o, _)| *o == outpoint)
            {
                found.push((outpoint, height));
            }
        }
    }
    found
}

/// [`find_past_the_gap`] and [`take_in_past_the_gap`] at once (tests).
#[cfg(test)]
pub fn adopt_addresses_past_the_gap(
    info: &mut key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    keys: &key_wallet::Wallet,
    paid: &[Address],
) -> Vec<Address> {
    let finds = find_past_the_gap(info, keys, paid);
    take_in_past_the_gap(info, finds)
}

/// BUG-232: the BIP44 account 0's change pool, its highest used and highest made index (0 when
/// none), for the journal.
pub fn change_pool_ends(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> (u32, u32) {
    use key_wallet::managed_account::address_pool::AddressPoolType;
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    info.accounts
        .standard_bip44_accounts
        .get(&0)
        .and_then(|a| {
            a.keys()
                .managed_account_type()
                .address_pools()
                .into_iter()
                .find(|p| p.pool_type == AddressPoolType::Internal)
                .map(|p| {
                    (
                        p.highest_used.unwrap_or_default(),
                        p.highest_generated.unwrap_or_default(),
                    )
                })
        })
        .unwrap_or_default()
}

/// The address pools of the live wallet's BIP44, BIP32 and CoinJoin accounts as the store keeps
/// them (BUG-194): every derived address of each pool, with whether it is used. platform-wallet
/// itself writes them only when it registers an account; [`crate::chain::Chain::save_pools`]
/// writes these.
pub fn pool_entries(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> Vec<platform_wallet::changeset::AccountAddressPoolEntry> {
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let a = info.accounts();
    a.standard_bip44_accounts
        .values()
        .chain(a.standard_bip32_accounts.values())
        .chain(a.coinjoin_accounts.values())
        .flat_map(|account| {
            let kind = account.keys().managed_account_type();
            let account_type = kind.to_account_type();
            kind.address_pools()
                .into_iter()
                .filter(|pool| !pool.addresses.is_empty())
                .map(
                    move |pool| platform_wallet::changeset::AccountAddressPoolEntry {
                        account_type,
                        pool_type: pool.pool_type,
                        addresses: pool.addresses.values().cloned().collect(),
                    },
                )
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The addresses of the live wallet's DashPay receiving accounts, each with the identities it
/// is between (BUG-194): the account of identity `to` for payments from its contact `from`, both
/// base58. Both identities are this wallet's when `from` is one of them: a payment between them.
pub fn own_contact_receipts(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> std::collections::HashMap<String, (String, String)> {
    use dpp::platform_value::string_encoding::Encoding;
    use dpp::prelude::Identifier;
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    let mut found = std::collections::HashMap::new();
    for (key, account) in info.accounts.dashpay_receival_accounts.iter() {
        let to = Identifier::from(key.user_identity_id).to_string(Encoding::Base58);
        let from = Identifier::from(key.friend_identity_id).to_string(Encoding::Base58);
        for pool in account.keys().managed_account_type().address_pools() {
            for info in pool.addresses.values() {
                found.insert(info.address.to_string(), (to.clone(), from.clone()));
            }
        }
    }
    found
}

/// A transfer between own addresses that paid a DashPay receiving address of one of this
/// wallet's identities from another of them (BUG-194: @1108c → @1108b, both in one wallet): one
/// line «Transfer between your identities», the fee only, with what moved. `ours`: this wallet's
/// identities (base58), each as History names it («@name», or its id); `receipts`:
/// [`own_contact_receipts`].
pub fn name_identity_transfers(
    history: &mut [(String, Activity)],
    receipts: &std::collections::HashMap<String, (String, String)>,
    ours: &std::collections::HashMap<String, String>,
) {
    for (_, a) in history.iter_mut() {
        if !matches!(a.kind, ActivityKind::SelfTransfer { .. }) {
            continue;
        }
        let Some(detail) = a.detail.as_deref() else {
            continue;
        };
        let found = detail.own_to.iter().find_map(|(address, value)| {
            let (to, from) = receipts.get(address)?;
            let named = |id: &String| ours.get(id).cloned().unwrap_or_else(|| id.clone());
            ours.contains_key(from)
                .then(|| (named(from), named(to), *value, address.clone()))
        });
        if let Some((from, to, moved, address)) = found {
            a.kind = ActivityKind::IdentityTransfer { from, to, moved };
            a.address = Some(address);
        }
    }
}

/// The wallet's coins by kind of account, how many and their sum in duffs (BUG-150, the journal):
/// BIP44, BIP32, DashPay receiving, CoinJoin.
pub fn coin_kinds(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> [(u32, u64); 4] {
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let a = info.accounts();
    let of = |accounts: Vec<&key_wallet::managed_account::ManagedCoreFundsAccount>| {
        accounts
            .iter()
            .flat_map(|x| x.utxos.values())
            .fold((0u32, 0u64), |(n, s), u| (n + 1, s + u.value()))
    };
    [
        of(a.standard_bip44_accounts.values().collect()),
        of(a.standard_bip32_accounts.values().collect()),
        of(a.dashpay_receival_accounts.values().collect()),
        of(a.coinjoin_accounts.values().collect()),
    ]
}

/// [`coin_kinds`] as a line of the journal.
pub fn coins_record(
    when: &'static str,
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> crate::diag::Record {
    let [bip44, bip32, dashpay, coinjoin] = coin_kinds(info);
    crate::diag::Record::Coins {
        when,
        bip44,
        bip32,
        dashpay,
        coinjoin,
        moved: None,
    }
}

/// The coins right after the unlock's [`rehome_filed_coins`], with how many it moved (BUG-148):
/// the line before it, `unlock`, shows the store's filing, this one what a payment sees from
/// then on. Numbers only.
pub fn rehomed_record(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
    moved: usize,
) -> crate::diag::Record {
    let mut record = coins_record("unlock_rehomed", info);
    if let crate::diag::Record::Coins { moved: m, .. } = &mut record {
        *m = Some(u32::try_from(moved).unwrap_or(u32::MAX));
    }
    record
}

/// The outpoints a Core payment can sign for: of the coins [`send_accounts`] hold, those whose
/// account derives their address (BUG-142).
pub fn signable_outpoints(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> std::collections::HashSet<key_wallet::dashcore::OutPoint> {
    use key_wallet::account::ManagedAccountTrait;
    send_accounts(info)
        .flat_map(|account| {
            account
                .utxos
                .values()
                .filter(|u| account.address_derivation_path(&u.address).is_some())
                .map(|u| u.outpoint)
        })
        .collect()
}

/// What of the Core balance no payment can sign for now (BUG-142): coins held only by an account
/// that does not derive their address — a contact's coin the store's early load filed in
/// account 0 (BUG-65) before its DashPay account has it. platform-wallet finds no key for them;
/// «Max» from Core leaves them out until the account that owns them holds them.
pub fn unsignable(info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo) -> Duffs {
    let signable = signable_outpoints(info);
    coins_once(info)
        .into_iter()
        .filter(|u| !signable.contains(&u.outpoint))
        .map(|u| u.value() as Duffs)
        .sum()
}

/// Moves each coin filed in BIP44 or BIP32 account 0 at an address that account does not derive
/// (BUG-148: a contact's coin the store's early load put there, BUG-65) to the DashPay receiving
/// account that derives it, once that account exists. The filters do not look at the coin's
/// block again, so it would stay where it was, [`unsignable`] for good. Coins no account derives
/// stay. Answers how many moved.
pub fn rehome_filed_coins(
    info: &mut key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> usize {
    use key_wallet::account::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let accounts = info.accounts_mut();
    // (account 0 of BIP44 or BIP32, the coin, the DashPay account that derives it)
    let mut moves = Vec::new();
    for (bip32, source) in [
        (false, accounts.standard_bip44_accounts.get(&0)),
        (true, accounts.standard_bip32_accounts.get(&0)),
    ] {
        let Some(source) = source else { continue };
        for utxo in source.utxos.values() {
            // Nor one held by a payment on its way: its reservation is the account's (the BUG-148
            // review, P3 2).
            if utxo.is_locked || source.address_derivation_path(&utxo.address).is_some() {
                continue;
            }
            let target = accounts
                .dashpay_receival_accounts
                .iter()
                .find(|(_, a)| a.address_derivation_path(&utxo.address).is_some());
            if let Some((key, _)) = target {
                moves.push((bip32, utxo.outpoint, *key));
            }
        }
    }
    let mut moved = 0;
    for (bip32, outpoint, key) in moves {
        let source = if bip32 {
            accounts.standard_bip32_accounts.get_mut(&0)
        } else {
            accounts.standard_bip44_accounts.get_mut(&0)
        };
        let Some(utxo) = source.and_then(|s| s.utxos.remove(&outpoint)) else {
            continue;
        };
        if let Some(target) = accounts.dashpay_receival_accounts.get_mut(&key) {
            target.mark_address_used(&utxo.address);
            target.utxos.entry(outpoint).or_insert(utxo);
            moved += 1;
        }
    }
    if moved > 0 {
        info.update_balance();
    }
    moved
}

/// The accounts a Core payment pays from (`SEND_FUNDING_SOURCES` at index 0): BIP44 and BIP32
/// account 0 and every DashPay receiving account.
fn send_accounts(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> impl Iterator<Item = &key_wallet::managed_account::ManagedCoreFundsAccount> {
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let accounts = info.accounts();
    accounts
        .standard_bip44_accounts
        .get(&0)
        .into_iter()
        .chain(accounts.standard_bip32_accounts.get(&0))
        .chain(accounts.dashpay_receival_accounts.values())
}

/// The wallet's Core balance as the snapshot shows it and payments check it: the coins of
/// [`coins_once`] — never platform-wallet's `balance()`, which counts a coin held by two
/// accounts twice. The same sum as key-wallet's total (confirmed, unconfirmed, immature and
/// locked coins).
pub fn core_balance(info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo) -> Duffs {
    coins_once(info).iter().map(|u| u.value() as Duffs).sum()
}

/// The wallet's coins (`values`, in duffs) as [`Fees::core_coins`]: largest first, each with
/// the fee of a payment spending it and every larger one — what platform-wallet's selection
/// (key-wallet `BranchAndBound`, which accumulates the largest coins when no change-free set
/// fits) pays for an amount those coins cover (BUG-55).
pub fn core_coins(mut values: Vec<u64>) -> Vec<CoreCoin> {
    values.sort_unstable_by(|a, b| b.cmp(a));
    values
        .into_iter()
        .enumerate()
        .map(|(n, value)| CoreCoin {
            value: value as Duffs,
            fee: fee_rate().calculate_fee(estimate_tx_size(n + 1, 1, true)) as Duffs,
        })
        .collect()
}

/// `core_payment`: the fee of a payment spending every coin (with change) — the most a
/// payment from this wallet can cost, so "Max" always leaves room for it. Platform fees are the
/// published costs.
pub fn fees(coins: usize) -> Fees {
    Fees {
        core_payment: fee_rate().calculate_fee(estimate_tx_size(coins.max(1), 1, true)) as Duffs,
        // The coins themselves: [`core_coins`], once the wallet's are read.
        core_unsignable: 0,
        core_coins: Vec::new(),
        // The final coins: the snapshot's, once read (SPV item 3).
        lock_coins: Vec::new(),
        // The latest protocol's, until the SDK says its own ([`lock_inputs_max`], Б2).
        lock_inputs_max: lock_inputs_max(dpp::version::PlatformVersion::latest()),
        credit_transfer: CREDIT_TRANSFER,
        credit_transfer_to_address: CREDIT_TRANSFER_TO_ADDRESS,
        nft_transfer: 30_000,
        // Documents are metered by Platform; until this data folder has paid for one of a kind
        // ([`platform_fees`] then takes what it really cost), what testnet charged
        // ([`tests::the_first_writes_are_priced_near_what_platform_charges`], BUG-54). A typical
        // write ("≈ N operations" on the credits tile, an app's request) is a contact request.
        document_write: CONTACT_REQUEST,
        identity_create: IDENTITY_CREATE,
        // The protocol's at 15 ([`identity_lock_min`]), until the SDK is up.
        identity_lock_min: 241_000,
        // The same, at 15 ([`top_up_lock_min`]).
        top_up_lock_min: 50_500,
        // FEAT-0015 stage 2: until measured, the architect's estimate; the threshold for one output at
        // 15 ([`address_fund_lock_min`]).
        address_fund_on_top: evolite_api::address_fund::ADDRESS_FUND_ON_TOP,
        address_fund_lock_min: 56_000,
        // Metered like the documents above: what remains of an identity's lock is shown
        // with a margin ([`Fees::name_on_top`]).
        name_regular: NAME_REGULAR,
        name_contested: 20_000_000,
        // Until the SDK speaks its version, the latest protocol's fund; the vote's durations need
        // the network too ([`contest_times`], set with every snapshot).
        name_contest_fund: contest_fund(dpp::version::PlatformVersion::latest()),
        name_vote_secs: 0,
        name_join_secs: 0,
        add_key: 10_000,
        key_add_unbound: evolite_api::key_plan::KEY_ADD_UNBOUND,
        key_add_bound: evolite_api::key_plan::KEY_ADD_BOUND,
        contact_request: CONTACT_REQUEST,
        contact_request_balance: CONTACT_REQUEST_BALANCE,
        // Not measured yet: one document like a contact request, with fewer indexes — this is
        // the upper side.
        profile: PROFILE,
        // The pool: Platform's own formulas at the SDK's protocol version ([`super::pool::fees`]),
        // set in the snapshot; these are the same formulas at protocol 15, until the SDK is up.
        pool_transfer: 114_140,
        shield: 149_760,
        shield_from_address: 114_140,
        shield_reserve: 228_280,
        unshield: 120_223,
        pool_withdrawal: 226_480,
        shield_from_core: 164_140,
        pool_top_up: 114_360,
        // The protocol's minimum at protocol 15 (`state_transition_min_fees.credit_withdrawal`),
        // until the SDK is up ([`platform_fees`]).
        credit_withdrawal: 400_000,
        credit_withdrawal_reserve: 400_000,
    }
}

/// The fund a contested name's claim pays at `version` (FEAT-0021, AC-T5): not returned to anyone
/// (АРХ-2). From protocol 14 it doubles at 250 contenders and every 50 after; a claim checks the
/// network's own figure before it is signed (R-2).
pub fn contest_fund(version: &dpp::version::PlatformVersion) -> Duffs {
    version
        .fee_version
        .vote_resolution_fund_fees
        .contested_document_vote_resolution_fund_required_amount
        .div_ceil(1000) as Duffs
}

/// The most inputs an asset lock may have at `version` (SPV Б2): Dash Core relays no larger one
/// and Platform validates the same bound (`max_asset_lock_transaction_inputs`).
pub fn lock_inputs_max(version: &dpp::version::PlatformVersion) -> usize {
    usize::from(
        version
            .dpp
            .state_transitions
            .identities
            .asset_locks
            .max_asset_lock_transaction_inputs,
    )
}

/// How long a masternode vote on a name lasts and how long others may join it, by `version` and
/// the network (FEAT-0021, АРХ-4, as DET's `contest_durations`): mainnet's production windows,
/// every other network's testing ones.
pub fn contest_times(fees: &mut Fees, version: &dpp::version::PlatformVersion, mainnet: bool) {
    let voting = &version.dpp.voting_versions;
    let joining = &version.dpp.validation.voting;
    let (vote_ms, join_ms) = if mainnet {
        (
            voting.default_vote_poll_time_duration_mainnet_ms,
            joining.allow_other_contenders_time_mainnet_ms,
        )
    } else {
        (
            voting.default_vote_poll_time_duration_test_network_ms,
            joining.allow_other_contenders_time_testing_ms,
        )
    };
    fees.name_vote_secs = vote_ms / 1000;
    fees.name_join_secs = join_ms / 1000;
}

/// The fees of Platform writes outside the pool, at protocol `version`. Withdrawals and documents
/// (profile, contact request, name) are metered by Platform: what the last one of a kind really
/// cost (the journal), else the protocol's minimum — for documents,
/// whose minimum is far below any real write, the estimate above. Creating an identity is
/// metered too ([`IDENTITY_CREATE`]); adding a key has a fixed protocol cost; a contested name
/// adds the masternode vote's fund.
pub fn platform_fees(
    fees: &mut Fees,
    version: &dpp::version::PlatformVersion,
    journal: &super::journal::Journal,
) {
    use super::journal::What;
    let duffs = |credits: u64| credits.div_ceil(1000) as Duffs;
    let min = &version.fee_version.state_transition_min_fees;
    // "At most" must not be below what is taken (BUG-108): the bound, or a quarter over the dearest
    // of the recent transfers to that kind of destination.
    let at_most = |bound: Duffs, to_a_name: bool| {
        let kind = |w: &What| matches!(w, What::Credits { to } if to.starts_with('@') == to_a_name);
        journal
            .dearest_fee(kind, RECENT_TRANSFERS)
            .map_or(bound, |fee| bound.max(fee.saturating_mul(5) / 4))
    };
    fees.credit_transfer = at_most(CREDIT_TRANSFER, true);
    fees.credit_transfer_to_address = at_most(CREDIT_TRANSFER_TO_ADDRESS, false);
    // BUG-197: a metered write is promised at no less than it may take. Platform's fee for a
    // document moves with the state of its trees (a contact answer promised at 73 936 took
    // 74 118), so the last fee alone is too little: the default, or a quarter over the dearest of
    // the last few writes of that kind — one rule for every fee the journal meters, as
    // `at_most` for transfers.
    let metered = |default: Duffs, kind: &dyn Fn(&What) -> bool| {
        journal
            .dearest_fee(kind, RECENT_WRITES)
            .map_or(default, |fee| default.max(fee.saturating_mul(5) / 4))
    };
    fees.credit_withdrawal = metered(duffs(min.credit_withdrawal), &|w| {
        matches!(w, What::Withdrawal { .. })
    });
    fees.credit_withdrawal_reserve = duffs(min.credit_withdrawal);
    fees.contact_request = metered(fees.contact_request, &|w| {
        matches!(w, What::ContactRequest { .. })
    });
    fees.profile = metered(fees.profile, &|w| matches!(w, What::Profile));
    fees.document_write = metered(fees.document_write, &|w| {
        matches!(w, What::ContactRequest { .. } | What::Profile)
    });
    fees.name_regular = metered(fees.name_regular, &|w| matches!(w, What::Name { .. }));
    fees.name_contest_fund = contest_fund(version);
    // BUG-225: a claim's writes (preorder + a contested domain) are dearer than a regular
    // name's: metered from the claims themselves, by the same rule, above the credits' bound.
    let claim_writes = metered(fees.name_credits_needed(), &|w| {
        matches!(w, What::NameClaim { .. })
    });
    fees.name_contested = claim_writes + fees.name_contest_fund;
    // An identity is made with six keys (`identity::new_identity_keys`).
    fees.identity_lock_min = identity_lock_min(version, 6);
    fees.top_up_lock_min = top_up_lock_min(version);
    fees.address_fund_lock_min = address_fund_lock_min(version);
    fees.add_key = duffs(min.identity_update + min.identity_key_in_creation_cost);
    (fees.key_add_bound, fees.key_add_unbound) = key_add_rates(journal);
}

/// What a new key costs, bound to a contract and not (BUG-183), by the rule of every metered
/// write (BUG-197, TECH-key-fee-rule): the default, or a quarter over the dearest key of the
/// last [`RECENT_WRITES`] changes of keys of that kind alone (all their new keys bound, or
/// none), when that is more. Before, a third over the last change alone: one cheap change
/// lowered the promise below a dearer one just before it.
pub fn key_add_rates(journal: &super::journal::Journal) -> (Duffs, Duffs) {
    use super::journal::Kind;
    use evolite_api::key_plan::{KEY_ADD_BOUND, KEY_ADD_UNBOUND};
    let entries = journal.entries();
    let dearest = |bound_kind: bool| {
        entries
            .iter()
            .rev()
            .filter_map(|e| match &e.kind {
                Some(Kind::KeysChanged { added, bound, .. })
                    if !added.is_empty()
                        && e.spent > 0
                        && if bound_kind {
                            *bound as usize == added.len()
                        } else {
                            *bound == 0
                        } =>
                {
                    Some(e.spent / added.len() as Duffs)
                }
                _ => None,
            })
            .take(RECENT_WRITES)
            .max()
    };
    let rate = |default: Duffs, measured: Option<Duffs>| {
        measured.map_or(default, |key| default.max(key.saturating_mul(5) / 4))
    };
    (
        rate(KEY_ADD_BOUND, dearest(true)),
        rate(KEY_ADD_UNBOUND, dearest(false)),
    )
}

#[cfg(test)]
mod tests {
    use super::super::sync::{Phase, Stage};
    use super::*;

    /// A coin of `value` duffs at `outpoint` (a txid of `seed`, output `vout`), confirmed.
    fn coin(seed: u8, vout: u32, value: u64) -> key_wallet::Utxo {
        use key_wallet::dashcore::hashes::Hash;
        let address = Address::new(
            key_wallet::Network::Testnet,
            key_wallet::dashcore::address::Payload::PubkeyHash(
                key_wallet::dashcore::PubkeyHash::from_byte_array([seed; 20]),
            ),
        );
        let mut utxo = key_wallet::Utxo::new(
            key_wallet::dashcore::OutPoint::new(
                key_wallet::dashcore::Txid::from_byte_array([seed; 32]),
                vout,
            ),
            key_wallet::dashcore::TxOut {
                value,
                script_pubkey: Default::default(),
            },
            address,
            1_570_000,
            false,
        );
        utxo.is_confirmed = true;
        utxo
    }

    /// BUG-77 (R-34): the store's early load puts a contact's DIP-15 coin into account 0, the
    /// filters find it again in its DashPay account — one outpoint in two accounts. The balance
    /// shown and checked counts it once; platform-wallet's sum counts it twice (the mutation
    /// «take `balance()`» is red here).
    #[test]
    fn a_coin_two_accounts_hold_is_counted_once() {
        use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
        use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
        let keys = super::super::keys_of(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        let main = coin(1, 0, 386_574_464 - 5_150_000);
        let contact = coin(2, 0, 5_050_000);
        let small = coin(3, 1, 100_000);
        {
            let mut accounts = info.accounts_mut().all_funding_accounts_mut();
            assert!(accounts.len() >= 2, "two funds accounts to hold one coin");
            for c in [&main, &contact, &small] {
                accounts[0].utxos.insert(c.outpoint, c.clone());
            }
            // The contact's coins found again in their own account.
            for c in [&contact, &small] {
                accounts[1].utxos.insert(c.outpoint, c.clone());
            }
        }
        info.update_balance();
        assert_eq!(info.balance().total(), 391_724_464, "upstream counts twice");
        assert_eq!(core_balance(&info), 386_574_464);
        assert_eq!(coins_once(&info).len(), 3);
        let mut values: Vec<u64> = coins_once(&info).iter().map(|u| u.value()).collect();
        values.sort_unstable();
        assert_eq!(values, [100_000, 5_050_000, 381_424_464]);
    }

    /// R-1 / BUG-26: dash-spv's "InstantSend" is InstantSend here only with a verified lock.
    #[test]
    fn an_instant_lock_counts_only_when_verified() {
        let locked = TransactionContext::InstantSend(Default::default());
        assert_eq!(proof(&locked, true), Proof::InstantLock);
        assert_eq!(proof(&locked, false), Proof::Mempool);
        assert_eq!(proof(&TransactionContext::Mempool, true), Proof::Mempool);
    }

    /// BUG-197 AC-T1, AC-T2: a metered write is promised at a quarter over the dearest of the
    /// last five of its kind, never below its default: a contact answer after 73 936 and 74 118
    /// is promised at 92 647 or more; so for a profile, any document, a name, a withdrawal.
    #[test]
    fn metered_writes_are_promised_with_a_margin() {
        use super::super::journal::{Entry, Journal, What};
        let dir = std::env::temp_dir().join(format!("evolite-metered-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Journal::new(dir.join("platform-history.json"));
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let mut defaults = fees(0);
        platform_fees(&mut defaults, v15, &journal);
        let paid = |at: i64, what: What, fee: Duffs| {
            // A withdrawal moves money: its fee is what was spent over what was moved.
            let moved = if matches!(what, What::Withdrawal { .. }) {
                1_000_000
            } else {
                0
            };
            journal.add(Entry {
                at,
                what,
                spent: moved + fee,
                moved,
                ..Default::default()
            })
        };
        paid(1, What::ContactRequest { to: "a".into() }, 73_936);
        paid(2, What::ContactRequest { to: "b".into() }, 74_118);
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        assert!(f.contact_request >= 74_118 * 5 / 4, "{}", f.contact_request);
        assert!(
            f.contact_request >= defaults.contact_request,
            "not below the default"
        );
        assert!(
            f.document_write >= 74_118 * 5 / 4,
            "any document: {}",
            f.document_write
        );
        paid(3, What::Profile, 50_000);
        paid(4, What::Name { label: "x".into() }, 60_000);
        paid(5, What::Withdrawal { to: "y".into() }, 900_000);
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        assert!(
            f.profile >= 62_500.max(defaults.profile),
            "profile {}",
            f.profile
        );
        assert!(f.name_regular >= 75_000, "name {}", f.name_regular);
        assert!(
            f.credit_withdrawal >= 1_125_000,
            "withdrawal {}",
            f.credit_withdrawal
        );
        // A cheap write does not lower it: the dearest of the last five.
        paid(6, What::ContactRequest { to: "c".into() }, 10_000);
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        assert!(f.contact_request >= 74_118 * 5 / 4, "{}", f.contact_request);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-54: before the first write of a kind, the estimate is within 30 % of what testnet
    /// charged (2026-11-05, manual run M-7) — a contact request was promised at a tenth of it —
    /// and the credits tile counts typical writes at that price.
    #[test]
    fn the_first_writes_are_priced_near_what_platform_charges() {
        let charged: Duffs = 102_028;
        let dir = std::env::temp_dir().join(format!("evolite-first-fees-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = super::super::journal::Journal::new(dir.join("platform-history.json"));
        let mut f = fees(0);
        platform_fees(
            &mut f,
            dpp::version::PlatformVersion::get(15).unwrap(),
            &journal,
        );
        let near = |estimate: Duffs| (estimate - charged).abs() * 10 <= charged * 3;
        assert!(
            near(f.contact_request),
            "contact request ≈ {}",
            f.contact_request
        );
        assert!(
            near(f.document_write),
            "a typical write ≈ {}",
            f.document_write
        );
        // A profile: 0.0004685 charged (BUG-171 P3), not priced as a contact request.
        let profile: Duffs = 46_850;
        assert!(
            f.profile >= profile && (f.profile - profile) * 10 <= profile * 4,
            "profile ≈ {}",
            f.profile
        );
        // werner555's 0.00449927 DASH of credits: about four contact requests, not 22.
        assert_eq!(449_927 / f.document_write, 4);
        // A regular name: 0.00057979 charged, promised at 0.0100 before part 2.
        let name: Duffs = 57_979;
        assert!(
            (f.name_regular - name).abs() * 10 <= name * 3,
            "name ≈ {}",
            f.name_regular
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-93, the estimate: the live creation with a name (@qa-kestrel-1108b, 2026-11-08) cost
    /// 222 937 duffs together; what goes on top covers it (the amount is never short) with a
    /// margin of at most 20 %, and is not the 106 000 of the protocol's minimum plus the name.
    #[test]
    fn bug_93_the_fees_on_top_cover_what_testnet_charged() {
        let dir = std::env::temp_dir().join(format!("evolite-bug93-est-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = super::super::journal::Journal::new(dir.join("platform-history.json"));
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        let on_top = f.identity_fees_on_top(true);
        let charged: Duffs = 222_937;
        assert!(on_top >= charged, "{on_top} on top, {charged} charged");
        assert!(
            (on_top - charged) * 100 <= charged * 20,
            "{on_top} on top, {charged} charged"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-93 (the customer): the amount typed is what ends up as credits; every fee is on top.
    /// The lock holds the amount and the fees, and the live charge leaves at least the amount.
    #[test]
    fn bug_93_the_lock_is_the_amount_and_the_fees_on_top() {
        let f = fees(0);
        let on_top = f.identity_fees_on_top(true) as u64;
        assert_eq!(identity_lock(500_000, &f, true), Some(500_000 + on_top));
        assert_eq!(
            identity_lock(500_000, &f, false),
            Some(500_000 + f.identity_create as u64)
        );
        let charged = 222_937;
        for typed in [100_000u64, 500_000, 30_000_000] {
            let lock = identity_lock(typed, &f, true).unwrap();
            assert!(lock - charged >= typed, "{typed} typed, {lock} locked");
        }
    }

    /// BUG-93: a top-up's lock is the amount and the fee on top, and the live fee (8 130 duffs
    /// on 0.01 DASH, 2026-11-05) leaves at least the amount. The review's must 1: Platform's
    /// least lock (50 500 at protocol 15) is reached at the least amount, and one duff below it
    /// no lock is made.
    #[test]
    fn bug_93_a_top_up_locks_the_amount_and_the_fee_on_top() {
        let dir = std::env::temp_dir().join(format!("evolite-bug93-top-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = super::super::journal::Journal::new(dir.join("platform-history.json"));
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        assert_eq!(
            f.top_up_lock_min, 50_500,
            "500 base + 50 000 processing start"
        );
        assert_eq!(
            fees(0).top_up_lock_min,
            50_500,
            "the same before the SDK is up"
        );
        assert_eq!(top_up_lock(1_000_000, &f), Some(1_010_000));
        assert!(top_up_lock(1_000_000, &f).unwrap() - 8_130 >= 1_000_000);
        assert_eq!(top_up_lock(40_500, &f), Some(50_500), "at the least");
        assert_eq!(top_up_lock(40_499, &f), None, "one duff below");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-0015 stage 2 (the architect, дизайнер-51): the least lock that funds a Platform
    /// address is 56 000 duffs at protocol 15 — 50 000 to start processing and 6 000 for the
    /// one output — and what the form and the core see before the SDK is up is the same.
    #[test]
    fn the_least_lock_for_an_address_is_the_protocols() {
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        assert_eq!(address_fund_lock_min(v15), 56_000);
        assert_eq!(fees(0).address_fund_lock_min, 56_000);
        assert_eq!(
            fees(0).address_fund_least(),
            35_000,
            "«At least 0,00035 DASH to the address» (56 000 less 21 000 on top, BUG-171 AC-T5)"
        );
    }

    /// BUG-93 review should 1: a `Continue` after a failed domain pays the preorder again
    /// (32 318 duffs at protocol 14, without a refund): the fees on top cover that too, with
    /// the longest legal name (about 7 500 more than the live 16 characters).
    #[test]
    fn bug_93_a_name_retried_still_leaves_the_amount() {
        let f = fees(0);
        let typed = 500_000;
        let lock = identity_lock(typed, &f, true).unwrap();
        assert!(lock - (222_937 + 32_318 + 7_500) >= typed, "{lock} locked");
    }

    /// BUG-97, the edge that matters: the least lock accepted for a named identity leaves, after
    /// the creation at its upper estimate (175 000; measured 145 715), enough for the first
    /// preorder and the domain's peak out of what remains: 165 000. Bare, the domain's peak is
    /// above the 78 546 Platform refused it at and at most the 117 786 it took it from.
    #[test]
    fn bug_97_the_least_named_lock_can_pay_for_the_name_after_the_creation() {
        let dir = std::env::temp_dir().join(format!("evolite-bug93-min-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = super::super::journal::Journal::new(dir.join("platform-history.json"));
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        let least_lock = |f: &Fees, named| {
            (1..2_000u64)
                .map(|k| k * 1_000)
                .find_map(|amount| identity_lock(amount, f, named))
                .expect("some lock is accepted")
        };
        let least = least_lock(&f, true);
        assert!(
            least >= 175_000 + 165_000,
            "the least lock {least} leaves too little for the name"
        );
        // A name this data folder paid little for does not lower the bar.
        f.name_regular = 1_000;
        assert!(least_lock(&f, true) >= 175_000 + 165_000);
        // Without a name there is nothing to pay after the creation: Platform's own minimum.
        assert!(least_lock(&f, false) >= 241_000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-54 part 2, review r1 must: Platform refuses an identity's asset lock below
    /// `calculate_min_required_fee` (protocol 3 on: base, the lock's processing start, six keys
    /// = 241 000 duffs at 15) and its DASH stay locked — such a lock is never made; one at the
    /// minimum is. BUG-93: with a name the least is higher still.
    #[test]
    fn an_identity_lock_is_never_below_what_platform_takes() {
        let dir = std::env::temp_dir().join(format!("evolite-lock-min-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = super::super::journal::Journal::new(dir.join("platform-history.json"));
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        assert_eq!(f.identity_lock_min, 241_000, "2 000 + 200 000 + 6 × 6 500");
        assert_eq!(
            fees(0).identity_lock_min,
            241_000,
            "the same before the SDK is up"
        );
        let least = f.identity_amount_least(true) as u64;
        assert_eq!(
            identity_lock(least, &f, true),
            Some(least + f.identity_fees_on_top(true) as u64),
            "at the least"
        );
        assert_eq!(identity_lock(least - 1, &f, true), None, "one duff below");
        assert_eq!(identity_lock(1, &f, true), None, "a tiny top-up");
        let nameless = f.identity_amount_least(false) as u64;
        assert_eq!(
            identity_lock(nameless, &f, false),
            Some(241_000),
            "exactly Platform's minimum"
        );
        assert_eq!(identity_lock(nameless - 1, &f, false), None, "below it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-108: before the first transfer "Fee, at most" said the protocol's minimum (100 duffs)
    /// and Platform took 2 901. Not below what it was seen to take, for either destination.
    #[test]
    fn at_most_a_first_transfer_is_not_below_what_platform_took() {
        let dir = std::env::temp_dir().join(format!("evolite-at-most-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = super::super::journal::Journal::new(dir.join("platform-history.json"));
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        assert!(f.credit_transfer >= 2_901, "{}", f.credit_transfer);
        assert!(
            f.credit_transfer_to_address >= 9_900,
            "{}",
            f.credit_transfer_to_address
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-108: a cheap transfer does not lower the next one's bound, and a dear one raises only
    /// its own destination's, with a margin over what was taken.
    #[test]
    fn at_most_follows_a_dearer_transfer_of_the_same_destination() {
        let dir = std::env::temp_dir().join(format!("evolite-at-most-2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = super::super::journal::Journal::new(dir.join("platform-history.json"));
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let mut f = fees(0);
        journal.add(super::super::journal::Entry {
            at: 1,
            what: super::super::journal::What::Credits { to: "@a".into() },
            spent: 10_100,
            moved: 10_000,
            ..Default::default()
        });
        platform_fees(&mut f, v15, &journal);
        assert_eq!(f.credit_transfer, CREDIT_TRANSFER, "cheap: still the bound");
        journal.add(super::super::journal::Entry {
            at: 2,
            what: super::super::journal::What::Credits {
                to: "tdash1x".into(),
            },
            spent: 110_000,
            moved: 10_000,
            ..Default::default()
        });
        platform_fees(&mut f, v15, &journal);
        assert_eq!(
            f.credit_transfer_to_address, 125_000,
            "100 000 and a quarter"
        );
        assert_eq!(f.credit_transfer, CREDIT_TRANSFER, "not the identity's");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-108: "at most" looks at the last transfers of a destination, not only the last: a
    /// cheap one after a dear one does not bring it down, and one reading gone wrong (a stale
    /// balance) does not raise it for good.
    #[test]
    fn at_most_holds_the_dearest_of_the_recent_transfers() {
        use super::super::journal::{Entry, Journal, What};
        let dir = std::env::temp_dir().join(format!("evolite-at-most-3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Journal::new(dir.join("platform-history.json"));
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let to_address = |at, fee: i64| Entry {
            at,
            what: What::Credits {
                to: "tdash1x".into(),
            },
            spent: 10_000 + fee,
            moved: 10_000,
            ..Default::default()
        };
        journal.add(to_address(1, 30_000));
        journal.add(to_address(2, 4_000));
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        assert_eq!(f.credit_transfer_to_address, 37_500, "30 000 and a quarter");
        for at in 3..=9 {
            journal.add(to_address(at, 4_000));
        }
        platform_fees(&mut f, v15, &journal);
        assert_eq!(
            f.credit_transfer_to_address, CREDIT_TRANSFER_TO_ADDRESS,
            "eight transfers on it is forgotten"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FEAT-0021 AC-T5: a contested name's fund and its vote's durations come from the protocol
    /// version and the network, not from figures in the code: 0.1 DASH from protocol 14 (0.2
    /// before), 14 days with 7 to join on mainnet, 90 minutes with 45 elsewhere.
    #[test]
    fn a_name_contest_takes_its_fund_and_times_from_the_protocol_and_the_network() {
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let v13 = dpp::version::PlatformVersion::get(13).unwrap();
        assert_eq!(contest_fund(v15), 10_000_000);
        assert_eq!(contest_fund(v13), 20_000_000);
        let mut f = fees(0);
        contest_times(&mut f, v15, true);
        assert_eq!(
            (f.name_vote_secs, f.name_join_secs),
            (14 * 86_400, 7 * 86_400)
        );
        contest_times(&mut f, v15, false);
        assert_eq!((f.name_vote_secs, f.name_join_secs), (90 * 60, 45 * 60));
        let journal = super::super::journal::Journal::new(
            std::env::temp_dir().join(format!("evolite-fund-{}.json", std::process::id())),
        );
        platform_fees(&mut f, v15, &journal);
        assert_eq!(f.name_contest_fund, 10_000_000);
        assert_eq!(
            f.name_contested,
            f.name_credits_needed() + f.name_contest_fund
        );
    }

    /// BUG-183 AC-T1: a key without a binding 24 000 duffs, DashPay's pair 80 000, only
    /// disabling the protocol's update; testnet's measures (17 428, 17 810 a key, 59 125 the pair)
    /// stay under the estimate less a quarter. AC-T3: App Connect's two bound keys and its
    /// document by the same rates.
    #[test]
    fn a_key_change_is_not_estimated_below_what_testnet_charged() {
        use evolite_api::key_plan::{KeyRequest, fee, fee_of_keys};
        let f = fees(0);
        assert_eq!(fee(&[KeyRequest::Tokens], &f), 24_000);
        assert_eq!(fee(&[KeyRequest::ReplaceMaster], &f), 24_000);
        assert_eq!(fee(&[KeyRequest::DashPay], &f), 80_000);
        assert_eq!(fee(&[], &f), f.add_key);
        let below = |estimate: Duffs| estimate - estimate / 4;
        for measured in [17_428, 17_810] {
            assert!(
                measured <= below(fee(&[KeyRequest::Transfers], &f)),
                "{measured}"
            );
        }
        assert!(59_125 <= below(fee(&[KeyRequest::DashPay], &f)));
        assert_eq!(
            fee_of_keys(2, 0, &f) + f.document_write,
            80_000 + f.document_write
        );
    }

    /// BUG-183 AC-T2: a pair that cost 70 000 lifts the bound rate to a third over 35 000 a key;
    /// a cheap change never takes a rate below the default.
    /// TECH-key-fee-rule: a key's rate is a quarter over the dearest of the recent changes of
    /// that kind, as every metered write — a cheaper change after a dearer one does not lower
    /// it; never below the default. Mutant: the last change alone, a third over (the old rule),
    /// gives 40 000 after the cheap one.
    #[test]
    fn a_key_change_learns_its_rate_from_the_dearest_recent_one() {
        use super::super::journal::{Journal, Kind, entry_of};
        let dir = std::env::temp_dir().join(format!("evolite-key-rates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Journal::new(dir.join("platform-history.json"));
        let change = |added: Vec<u32>, bound: u32, spent: Duffs| {
            entry_of(
                Kind::KeysChanged {
                    added,
                    disabled: Vec::new(),
                    bound,
                },
                1_793_000_000,
                spent,
                None,
            )
        };
        assert_eq!(key_add_rates(&journal), (40_000, 24_000));
        journal.add(change(vec![6, 7], 2, 70_000));
        journal.add(change(vec![8], 0, 9_000));
        // 35 000 a bound key, a quarter over: 43 750; the unbound one below its default.
        assert_eq!(key_add_rates(&journal), (43_750, 24_000));
        // A cheaper bound change after it: the dearest of the recent ones still counts.
        journal.add(change(vec![9], 1, 30_000));
        assert_eq!(key_add_rates(&journal), (43_750, 24_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// TECH-key-fee-rule: App Connect's keys (bound to the app's contract, `fee_of_keys`) and the
    /// keys screen's (`key_plan::fee`) cost the same for the same keys, from the same measured
    /// fees (BUG-236: App Connect prices with them too).
    #[test]
    fn app_connect_and_the_keys_screen_price_keys_alike() {
        use super::super::journal::{Journal, Kind, entry_of};
        use evolite_api::key_plan::{self, KeyRequest};
        let dir = std::env::temp_dir().join(format!("evolite-key-alike-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Journal::new(dir.join("platform-history.json"));
        journal.add(entry_of(
            Kind::KeysChanged {
                added: vec![6, 7],
                disabled: Vec::new(),
                bound: 2,
            },
            1_793_000_000,
            80_000,
            None,
        ));
        let mut f = fees(0);
        platform_fees(&mut f, dpp::version::PlatformVersion::latest(), &journal);
        assert_eq!(f.key_add_bound, 50_000, "a quarter over 40 000 a key");
        let screen = key_plan::fee(&[KeyRequest::DashPay], &f);
        let app_connect = key_plan::fee_of_keys(2, 0, &f);
        assert_eq!(screen, 100_000);
        assert_eq!(app_connect, screen);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn platform_fees_follow_the_protocol_and_the_last_write() {
        let dir = std::env::temp_dir().join(format!("evolite-fees-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = super::super::journal::Journal::new(dir.join("platform-history.json"));
        let v15 = dpp::version::PlatformVersion::get(15).unwrap();
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        // Nothing written yet: the protocol's own numbers.
        assert_eq!(f.credit_withdrawal, 400_000);
        assert_eq!(f.credit_withdrawal_reserve, 400_000);
        assert_eq!(f.credit_transfer, CREDIT_TRANSFER);
        assert_eq!(f.credit_transfer_to_address, CREDIT_TRANSFER_TO_ADDRESS);
        assert_eq!(
            f.identity_create, IDENTITY_CREATE,
            "metered, not the minimum"
        );

        assert_eq!(f.add_key, 6_600);
        assert_eq!(
            f.name_contested,
            f.name_credits_needed() + 10_000_000,
            "the claim's bound plus the vote fund"
        );
        // After a name and a transfer: what they really cost.
        journal.add(super::super::journal::Entry {
            at: 1,
            what: super::super::journal::What::Name { label: "a".into() },
            spent: 123_456,
            moved: 0,
            ..Default::default()
        });
        journal.add(super::super::journal::Entry {
            at: 2,
            what: super::super::journal::What::Credits { to: "@b".into() },
            spent: 10_777,
            moved: 10_000,
            ..Default::default()
        });
        // A name: a quarter over what it took, never below the default (BUG-197); a cheap
        // transfer: still the bound.
        let mut f = fees(0);
        let name_default = f.name_regular;
        platform_fees(&mut f, v15, &journal);
        assert_eq!(
            (f.name_regular, f.credit_transfer),
            (name_default.max(123_456 * 5 / 4), CREDIT_TRANSFER)
        );
        // BUG-225: a claim that wrote 107 336 (its fund apart) and one dearer: the estimate is
        // a quarter over the dearest, above the bound; N2 and N2t show this one figure.
        for (at, writes) in [(3, 107_336), (4, 1_400_000)] {
            journal.add(super::super::journal::Entry {
                at,
                what: super::super::journal::What::NameClaim { label: "q".into() },
                spent: 10_000_000 + writes,
                moved: 10_000_000,
                ..Default::default()
            });
        }
        let mut f = fees(0);
        platform_fees(&mut f, v15, &journal);
        assert_eq!(f.name_claim_writes(), 1_400_000 * 5 / 4);
        assert!(f.name_claim_writes() >= f.name_credits_needed());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// BUG-232 (QA batch 31, qa-f31's tx1): a payment between own identities kept in the store
    /// as the contact account's slice: it lists only the 10 000 to our own DashPay address, not
    /// the change. Once its input is ours, the unlisted change is ours by the same rule as
    /// `own_outputs` (the store's pools here; the live ones may end before it): a transfer
    /// between own addresses, the fee only. The twin: the same change address not ours (a
    /// contact's address we pay, not in either pool) stays paid out.
    #[test]
    fn an_unlisted_change_of_ours_is_not_paid_out() {
        use key_wallet::dashcore::address::Payload;
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::dashcore::{OutPoint, PubkeyHash, Transaction, TxIn, TxOut};
        use key_wallet::managed_account::transaction_record::OutputDetail;
        let address = |n: u8| {
            Address::new(
                key_wallet::Network::Testnet,
                Payload::PubkeyHash(PubkeyHash::from_byte_array([n; 20])),
            )
        };
        let (coin_at, own_dashpay, change_at) = (address(1), address(2), address(3));
        let (to_us, change, fee) = (10_000u64, 14_525_584u64, 226u64);
        let funding_tx = Transaction {
            version: 2,
            lock_time: 0,
            input: Vec::new(),
            output: vec![TxOut {
                value: to_us + change + fee,
                script_pubkey: coin_at.script_pubkey(),
            }],
            special_transaction_payload: None,
        };
        let payment_tx = Transaction {
            version: 3,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: OutPoint::new(funding_tx.txid(), 0),
                ..TxIn::default()
            }],
            output: vec![
                TxOut {
                    value: to_us,
                    script_pubkey: own_dashpay.script_pubkey(),
                },
                TxOut {
                    value: change,
                    script_pubkey: change_at.script_pubkey(),
                },
            ],
            special_transaction_payload: None,
        };
        let records = || {
            let funding = TransactionRecord::new(
                funding_tx.clone(),
                key_wallet::account::AccountType::Standard {
                    index: 0,
                    standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
                },
                TransactionContext::Mempool,
                TransactionType::Standard,
                TransactionDirection::Incoming,
                Vec::new(),
                vec![OutputDetail {
                    index: 0,
                    role: OutputRole::Received,
                    address: Some(coin_at.clone()),
                    value: to_us + change + fee,
                }],
                (to_us + change + fee) as i64,
            );
            // The contact account's slice, as the store kept it.
            let slice = TransactionRecord::new(
                payment_tx.clone(),
                key_wallet::account::AccountType::DashpayReceivingFunds {
                    index: 0,
                    user_identity_id: [1; 32],
                    friend_identity_id: [2; 32],
                },
                TransactionContext::Mempool,
                TransactionType::Standard,
                TransactionDirection::Incoming,
                Vec::new(),
                vec![OutputDetail {
                    index: 0,
                    role: OutputRole::Received,
                    address: Some(own_dashpay.clone()),
                    value: to_us,
                }],
                to_us as i64,
            );
            vec![slice, funding]
        };

        // Ours: the change address is in the store's internal pool.
        let mut ours = records();
        attribute_late_inputs(&mut ours, |a| {
            (*a == change_at).then_some(OutputRole::Change)
        });
        let line = activity(&ours[0], 0, true);
        assert!(
            matches!(line.kind, ActivityKind::SelfTransfer { .. }),
            "{:?}",
            line.kind
        );
        assert_eq!(line.amount, -(fee as i64), "the fee only");

        // Not ours (a contact's address we pay): paid out, as before.
        let mut theirs = records();
        attribute_late_inputs(&mut theirs, |_| None);
        let line = activity(&theirs[0], 0, true);
        assert!(
            matches!(&line.kind, ActivityKind::Sent { to } if *to == change_at.to_string()),
            "{:?}",
            line.kind
        );
        assert_eq!(line.amount, -((change + fee) as i64));
    }

    /// BUG-232 (QA batch 36, qa-f31's tx1 and tx2): both kept in the store as a contact
    /// account's slice, each listing only its 10 000 to our own DashPay address. tx2 spends
    /// tx1's change, unlisted in tx1's slice: by the same rule of «ours» it is our coin spent, so
    /// tx2 is a transfer between own addresses too, the fee only — not «received 0.0001».
    #[test]
    fn a_spend_of_an_unlisted_change_of_ours_is_ours_too() {
        use key_wallet::dashcore::address::Payload;
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::dashcore::{OutPoint, PubkeyHash, Transaction, TxIn, TxOut};
        use key_wallet::managed_account::transaction_record::OutputDetail;
        let address = |n: u8| {
            Address::new(
                key_wallet::Network::Testnet,
                Payload::PubkeyHash(PubkeyHash::from_byte_array([n; 20])),
            )
        };
        let (coin_at, dashpay_1, change_1, dashpay_2, change_2) =
            (address(1), address(2), address(3), address(4), address(5));
        let (to_us, fee) = (10_000u64, 226u64);
        let funded = 14_545_810u64;
        let tx = |from: OutPoint, outputs: &[(&Address, u64)]| Transaction {
            version: 3,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: from,
                ..TxIn::default()
            }],
            output: outputs
                .iter()
                .map(|(a, value)| TxOut {
                    value: *value,
                    script_pubkey: a.script_pubkey(),
                })
                .collect(),
            special_transaction_payload: None,
        };
        let funding_tx = tx(
            OutPoint::new(key_wallet::dashcore::Txid::from_byte_array([9; 32]), 0),
            &[(&coin_at, funded)],
        );
        let change_1_value = funded - to_us - fee;
        let tx1 = tx(
            OutPoint::new(funding_tx.txid(), 0),
            &[(&dashpay_1, to_us), (&change_1, change_1_value)],
        );
        let change_2_value = change_1_value - to_us - fee;
        let tx2 = tx(
            OutPoint::new(tx1.txid(), 1),
            &[(&dashpay_2, to_us), (&change_2, change_2_value)],
        );
        let record = |t: &Transaction, account, listed: Vec<OutputDetail>, net| {
            TransactionRecord::new(
                t.clone(),
                account,
                TransactionContext::Mempool,
                TransactionType::Standard,
                TransactionDirection::Incoming,
                Vec::new(),
                listed,
                net,
            )
        };
        let bip44 = key_wallet::account::AccountType::Standard {
            index: 0,
            standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
        };
        let slice = || key_wallet::account::AccountType::DashpayReceivingFunds {
            index: 0,
            user_identity_id: [1; 32],
            friend_identity_id: [2; 32],
        };
        let received = |index, at: &Address, value| OutputDetail {
            index,
            role: OutputRole::Received,
            address: Some(at.clone()),
            value,
        };
        // A third, spending tx2's change the same way.
        let (dashpay_3, change_3) = (address(6), address(7));
        let tx3 = tx(
            OutPoint::new(tx2.txid(), 1),
            &[
                (&dashpay_3, to_us),
                (&change_3, change_2_value - to_us - fee),
            ],
        );
        // Stored newest first (review condition 2): the rule is by address, not by what was
        // judged before.
        let records = || {
            vec![
                record(
                    &tx3,
                    slice(),
                    vec![received(0, &dashpay_3, to_us)],
                    to_us as i64,
                ),
                record(
                    &tx2,
                    slice(),
                    vec![received(0, &dashpay_2, to_us)],
                    to_us as i64,
                ),
                record(
                    &tx1,
                    slice(),
                    vec![received(0, &dashpay_1, to_us)],
                    to_us as i64,
                ),
                record(
                    &funding_tx,
                    bip44,
                    vec![received(0, &coin_at, funded)],
                    funded as i64,
                ),
            ]
        };
        let mut ours = records();
        let changes = [change_1.clone(), change_2.clone(), change_3.clone()];
        attribute_late_inputs(&mut ours, |a| {
            changes.contains(a).then_some(OutputRole::Change)
        });
        for (i, name) in [(2, "tx1"), (1, "tx2"), (0, "tx3")] {
            let line = activity(&ours[i], 0, true);
            assert!(
                matches!(line.kind, ActivityKind::SelfTransfer { .. }),
                "{name}: {:?}",
                line.kind
            );
            assert_eq!(line.amount, -(fee as i64), "{name}: the fee only");
        }

        // The twin (condition 3): tx1's second output paid a contact's address, not in our
        // pools — tx2 spends no coin of ours: it is what came to us.
        let mut theirs = records();
        attribute_late_inputs(&mut theirs, |a| {
            [change_2.clone(), change_3.clone()]
                .contains(a)
                .then_some(OutputRole::Change)
        });
        let tx2_line = activity(&theirs[1], 0, true);
        assert!(
            matches!(tx2_line.kind, ActivityKind::Received { .. }),
            "{:?}",
            tx2_line.kind
        );
        assert!(theirs[1].input_details.is_empty(), "its input is not ours");
    }

    /// BUG-239 (QA batch 37, qa-f37's tx 6a1e14…): a payment from a contact to us, spending
    /// the contact's coins at the addresses we pay them at — coins we sent them. This run's
    /// records include the contact's watch account (`DashpayExternalAccount`): it lists our
    /// payment's output to the contact as received and the contact's spend as its inputs. The
    /// line is what came to us, not a transfer between own addresses.
    #[test]
    fn a_contacts_payment_from_coins_we_sent_them_is_received() {
        use key_wallet::dashcore::address::Payload;
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::dashcore::{OutPoint, PubkeyHash, Transaction, TxIn, TxOut};
        use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};
        let address = |n: u8| {
            Address::new(
                key_wallet::Network::Testnet,
                Payload::PubkeyHash(PubkeyHash::from_byte_array([n; 20])),
            )
        };
        let (our_coin, their_address, our_change, our_dashpay) =
            (address(1), address(2), address(3), address(4));
        let tx = |from: OutPoint, outputs: &[(&Address, u64)]| Transaction {
            version: 3,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: from,
                ..TxIn::default()
            }],
            output: outputs
                .iter()
                .map(|(a, value)| TxOut {
                    value: *value,
                    script_pubkey: a.script_pubkey(),
                })
                .collect(),
            special_transaction_payload: None,
        };
        let bip44 = key_wallet::account::AccountType::Standard {
            index: 0,
            standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
        };
        let watch = key_wallet::account::AccountType::DashpayExternalAccount {
            index: 0,
            user_identity_id: [1; 32],
            friend_identity_id: [2; 32],
        };
        let receiving = key_wallet::account::AccountType::DashpayReceivingFunds {
            index: 0,
            user_identity_id: [1; 32],
            friend_identity_id: [2; 32],
        };
        let detail = |index, role, at: &Address, value| OutputDetail {
            index,
            role,
            address: Some(at.clone()),
            value,
        };
        let record = |t: &Transaction, account, inputs: Vec<InputDetail>, outputs, direction| {
            TransactionRecord::new(
                t.clone(),
                account,
                TransactionContext::Mempool,
                TransactionType::Standard,
                direction,
                inputs,
                outputs,
                0,
            )
        };
        // Ours to the contact: 0.001 to their address, the change back.
        let funding = OutPoint::new(key_wallet::dashcore::Txid::from_byte_array([9; 32]), 0);
        let paid = tx(
            funding,
            &[(&their_address, 100_000), (&our_change, 899_774)],
        );
        let spent = InputDetail {
            index: 0,
            value: 1_000_000,
            address: our_coin.clone(),
        };
        // The contact's payment to us from that coin.
        let theirs = tx(OutPoint::new(paid.txid(), 0), &[(&our_dashpay, 99_774)]);
        let watched_input = InputDetail {
            index: 0,
            value: 100_000,
            address: their_address.clone(),
        };
        let live = [
            record(
                &paid,
                bip44,
                vec![spent],
                vec![
                    detail(0, OutputRole::Sent, &their_address, 100_000),
                    detail(1, OutputRole::Change, &our_change, 899_774),
                ],
                TransactionDirection::Outgoing,
            ),
            record(
                &paid,
                watch,
                vec![],
                vec![detail(0, OutputRole::Received, &their_address, 100_000)],
                TransactionDirection::Incoming,
            ),
            record(
                &theirs,
                watch,
                vec![watched_input],
                vec![],
                TransactionDirection::Outgoing,
            ),
            record(
                &theirs,
                receiving,
                vec![],
                vec![detail(0, OutputRole::Received, &our_dashpay, 99_774)],
                TransactionDirection::Incoming,
            ),
        ];
        let mut records = one_per_transaction(live.iter());
        attribute_late_inputs(&mut records, |_| None);
        let line_of = |txid| {
            let r = records.iter().find(|r| r.txid == txid).unwrap();
            activity(r, 0, true)
        };
        let line = line_of(theirs.txid());
        assert!(
            matches!(line.kind, ActivityKind::Received { .. }),
            "{:?}",
            line.kind
        );
        assert_eq!(line.amount, 99_774, "what came to us");
        let ours = line_of(paid.txid());
        assert!(
            matches!(ours.kind, ActivityKind::Sent { .. }),
            "our payment to them stays sent: {:?}",
            ours.kind
        );
    }

    /// BUG-232 review (QA batch 31) condition 2: the live side of «ours» leaves out the
    /// addresses we pay a contact at (`DashpayExternalAccount`), so a payment to a contact never
    /// becomes a transfer between own; our own contact-receiving account's are in.
    #[test]
    fn a_contacts_address_we_pay_is_not_ours_live() {
        use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
        use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
        let keys = crate::keys_of(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let mut info = key_wallet::wallet::ManagedWalletInfo::from_wallet(&keys, 0);
        // Keys of another phrase: no address of the two can be one of this wallet's own.
        let other = crate::keys_of(
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
        )
        .unwrap();
        let account = |account_type, xpub| key_wallet::Account {
            parent_wallet_id: None,
            account_type,
            network: key_wallet::Network::Testnet,
            account_xpub: xpub,
            is_watch_only: false,
        };
        let theirs = key_wallet::managed_account::ManagedCoreFundsAccount::from_account(&account(
            key_wallet::account::AccountType::DashpayExternalAccount {
                index: 0,
                user_identity_id: [1; 32],
                friend_identity_id: [2; 32],
            },
            other.accounts.coinjoin_accounts[&0].account_xpub,
        ));
        let mine = key_wallet::managed_account::ManagedCoreFundsAccount::from_account(&account(
            key_wallet::account::AccountType::DashpayReceivingFunds {
                index: 0,
                user_identity_id: [1; 32],
                friend_identity_id: [3; 32],
            },
            other.accounts.standard_bip44_accounts[&0].account_xpub,
        ));
        let their_address = theirs.managed_account_type().all_addresses()[0].clone();
        let my_address = mine.managed_account_type().all_addresses()[0].clone();
        info.accounts_mut()
            .insert_funds_bearing_account(theirs)
            .unwrap();
        info.accounts_mut()
            .insert_funds_bearing_account(mine)
            .unwrap();
        let own = live_own(&info);
        assert_eq!(own.get(&their_address.script_pubkey()), None);
        assert_eq!(
            own.get(&my_address.script_pubkey()),
            Some(&OutputRole::Received)
        );
    }

    /// A restored wallet: the payment to a contact (with change) was recorded before the coin
    /// it spends was found, from its change alone. With the coin's record beside it, it is
    /// "Sent" to the contact's key address again, with its fee — and the contact counts it.
    #[test]
    fn a_restored_payment_to_a_contact_with_change_is_sent_not_received() {
        use key_wallet::dashcore::address::Payload;
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::dashcore::{OutPoint, PubkeyHash, ScriptBuf, Transaction, TxIn, TxOut};
        use key_wallet::managed_account::transaction_record::OutputDetail;

        let address = |n: u8| {
            Address::new(
                key_wallet::Network::Testnet,
                Payload::PubkeyHash(PubkeyHash::from_byte_array([n; 20])),
            )
        };
        let (coin_at, contact_key, change_at) = (address(1), address(2), address(3));
        let tx = |input: Vec<TxIn>, output: Vec<TxOut>| Transaction {
            version: 2,
            lock_time: 0,
            input,
            output,
            special_transaction_payload: None,
        };
        let out = |value, at: &Address| TxOut {
            value,
            script_pubkey: at.script_pubkey(),
        };
        let record =
            |transaction: Transaction, direction, output_details: Vec<OutputDetail>, net_amount| {
                let mut r = TransactionRecord::new(
                    transaction,
                    key_wallet::account::AccountType::Standard {
                        index: 0,
                        standard_account_type:
                            key_wallet::account::StandardAccountType::BIP44Account,
                    },
                    TransactionContext::Mempool,
                    TransactionType::Standard,
                    direction,
                    Vec::new(),
                    output_details,
                    net_amount,
                );
                r.fee = None;
                r
            };
        // The coin: 998 000 duffs to our address.
        let funding = tx(Vec::new(), vec![out(998_000, &coin_at)]);
        let coin = OutPoint::new(funding.txid(), 0);
        let funding = record(
            funding,
            TransactionDirection::Incoming,
            vec![OutputDetail {
                index: 0,
                role: OutputRole::Received,
                address: Some(coin_at.clone()),
                value: 998_000,
            }],
            998_000,
        );
        // The payment: 100 000 to the contact's key address, 897 774 back as change, 226 fee —
        // recorded with only the change, as the restore saw it.
        let payment = tx(
            vec![TxIn {
                previous_output: coin,
                script_sig: ScriptBuf::new(),
                sequence: u32::MAX,
                witness: Default::default(),
            }],
            vec![out(100_000, &contact_key), out(897_774, &change_at)],
        );
        let payment = record(
            payment,
            TransactionDirection::Incoming,
            vec![OutputDetail {
                index: 1,
                role: OutputRole::Change,
                address: Some(change_at.clone()),
                value: 897_774,
            }],
            897_774,
        );
        let mut records = vec![payment, funding];
        attribute_late_inputs(&mut records, |_| None);
        let line = activity(&records[0], 0, true);
        assert_eq!(line.amount, -100_226, "what left the wallet");
        assert_eq!(line.fee, Some(226));
        assert!(
            matches!(&line.kind, ActivityKind::Sent { to } if *to == contact_key.to_string()),
            "{:?}",
            line.kind
        );
        // The coin's own line is as it was.
        assert_eq!(activity(&records[1], 0, true).amount, 998_000);

        let contact = Contact {
            identity_id: "Jonas111".into(),
            name: Some("bob".into()),
            profile: Default::default(),
            state: ContactState::Established,
            since: 0,
            payments: 0,
            last_payment: None,
            channel_broken: false,
        };
        let keys = [(
            contact_key.to_string(),
            ("Me111".to_owned(), "Jonas111".to_owned()),
        )]
        .into();
        let mut history = vec![line];
        name_contact_payments(&mut history, &keys, std::slice::from_ref(&contact));
        assert!(matches!(&history[0].kind, ActivityKind::Sent { to } if to == "@bob"));
        let mut contacts = vec![contact];
        count_contact_payments(&history, &keys, Some("Me111"), &mut contacts);
        assert_eq!(contacts[0].payments, 1);
    }

    #[test]
    fn a_payment_to_a_contact_key_names_the_contact() {
        let sent = |to: &str| Activity {
            id: 1,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at: 0,
            amount: -10_226,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: layer::CORE,
            flow: Flow::Out,
            key: String::new(),
            detail: None,
            note: None,
            rate: None,
            tax_transfer: false,
        };
        let contact = |id: &str, name: Option<&str>| Contact {
            identity_id: id.into(),
            name: name.map(Into::into),
            profile: Default::default(),
            state: ContactState::Established,
            since: 0,
            payments: 0,
            last_payment: None,
            channel_broken: false,
        };
        let keys = [
            (
                "yLcB1".to_owned(),
                ("Me111".to_owned(), "Abc111".to_owned()),
            ),
            (
                "yev72".to_owned(),
                ("Me111".to_owned(), "Xyz9999888".to_owned()),
            ),
        ]
        .into();
        let contacts = [
            contact("Abc111", Some("carol")),
            contact("Xyz9999888", None),
        ];
        let mut h = vec![sent("yLcB1"), sent("yev72"), sent("yOther")];
        name_contact_payments(&mut h, &keys, &contacts);
        let to: Vec<_> = h
            .iter()
            .map(|a| match &a.kind {
                ActivityKind::Sent { to } => (to.as_str(), a.address.as_deref()),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            to,
            [
                ("@carol", Some("yLcB1")),
                ("Xyz9…9888", Some("yev72")),
                ("yOther", None)
            ]
        );
        let mut contacts = contacts;
        count_contact_payments(&h, &keys, Some("Me111"), &mut contacts);
        assert_eq!((contacts[0].payments, contacts[1].payments), (1, 1));
    }

    /// BUG-196 AC-T1, AC-T2: one wallet, our identities A and B, a contact C of both, three
    /// payments A → C. A's C: 3, the last of them; B's C: none and no «last» — on record in
    /// platform-wallet or not (a restore: none on record).
    #[test]
    fn payments_to_a_contact_are_counted_for_the_identity_that_paid() {
        let sent = |at: i64, address: &str| Activity {
            id: 1,
            kind: ActivityKind::Sent {
                to: "@carol".into(),
            },
            chain: Chain::Core,
            at,
            amount: -10_000,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: Some(address.into()),
            layers: layer::CORE,
            flow: Flow::Out,
            key: String::new(),
            detail: None,
            note: None,
            rate: None,
            tax_transfer: false,
        };
        let carol = |payments: u32| Contact {
            identity_id: "C111".into(),
            name: Some("carol".into()),
            profile: Default::default(),
            state: ContactState::Established,
            since: 0,
            payments,
            last_payment: None,
            channel_broken: false,
        };
        // A's account for C, and B's: their own addresses.
        let keys: std::collections::HashMap<String, (String, String)> = [
            ("yA1".to_owned(), ("A111".to_owned(), "C111".to_owned())),
            ("yA2".to_owned(), ("A111".to_owned(), "C111".to_owned())),
            ("yB1".to_owned(), ("B111".to_owned(), "C111".to_owned())),
        ]
        .into();
        let history = vec![sent(100, "yA1"), sent(200, "yA2"), sent(300, "yA1")];
        // On record in platform-wallet (by the pair: A 3, B 0), and after a restore (0, 0).
        for (on_record_a, on_record_b) in [(3, 0), (0, 0)] {
            let mut a = vec![carol(on_record_a)];
            count_contact_payments(&history, &keys, Some("A111"), &mut a);
            assert_eq!((a[0].payments, a[0].last_payment), (3, Some(300)), "A");
            let mut b = vec![carol(on_record_b)];
            count_contact_payments(&history, &keys, Some("B111"), &mut b);
            assert_eq!((b[0].payments, b[0].last_payment), (0, None), "B");
        }
    }

    #[test]
    fn a_chainlock_settles_the_blocks_under_it() {
        let at = |block: Option<u32>, proof: Proof| Activity {
            id: 1,
            kind: ActivityKind::Received { from: None },
            chain: Chain::Core,
            at: 0,
            amount: 1,
            proof,
            txid: None,
            fee: None,
            block,
            address: None,
            layers: layer::CORE,
            flow: Flow::In,
            key: String::new(),
            detail: None,
            note: None,
            rate: None,
            tax_transfer: false,
        };
        let mut h = vec![
            at(Some(900), Proof::InBlock),
            at(Some(1000), Proof::InBlock),
            at(Some(1001), Proof::InBlock),
            at(None, Proof::Mempool),
            at(Some(800), Proof::InstantLock),
        ];
        settle(&mut h, Some(1000));
        let proofs: Vec<_> = h.iter().map(|a| a.proof).collect();
        assert_eq!(
            proofs,
            [
                Proof::ChainLock,
                Proof::ChainLock,
                Proof::InBlock,
                Proof::Mempool,
                Proof::InstantLock
            ]
        );
        settle(&mut h, None);
        assert_eq!(h[2].proof, Proof::InBlock);
    }

    #[test]
    fn pays_only_testnet_addresses() {
        assert!(payee(" yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ ").is_ok());
        // A well-formed address: refused for its network, not for its checksum.
        assert!(
            "XpESxaUmonkq8RaLLp46Brx2K39ggQe226"
                .parse::<Address<NetworkUnchecked>>()
                .is_ok()
        );
        // A mainnet address of the same key would send real money elsewhere.
        assert_eq!(
            payee("XpESxaUmonkq8RaLLp46Brx2K39ggQe226").err(),
            Some(CoreError::BadAddress)
        );
        assert_eq!(payee("not an address").err(), Some(CoreError::BadAddress));
        // A typo (one letter changed) is no address at all; mainnet is an address, elsewhere.
        assert!(not_an_address("yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXR").is_some());
        assert!(not_an_address("not an address").is_some());
        assert_eq!(not_an_address("XpESxaUmonkq8RaLLp46Brx2K39ggQe226"), None);
        assert_eq!(not_an_address(" yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ "), None);
    }

    /// R-3: a witness address parses, and Dash has no witness outputs that a wallet could spend:
    /// money sent there is gone. P2PKH and P2SH are the two kinds of Core address there are.
    #[test]
    fn pays_only_addresses_a_dash_wallet_can_spend() {
        use key_wallet::dashcore::address::Payload;
        use key_wallet::dashcore::bech32::{self, ToBase32, Variant, u5};
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::dashcore::{Network, ScriptHash};
        // As an address of a testnet witness program is written: `td1…`. (The library prints them
        // with `tb1`, which it does not read back.)
        let witness = |version: u8, length: usize, variant| {
            let mut data = vec![u5::try_from_u8(version).unwrap()];
            data.extend(vec![1u8; length].to_base32());
            bech32::encode("td", data, variant).unwrap()
        };
        for to in [
            witness(0, 20, Variant::Bech32),
            witness(0, 32, Variant::Bech32),
            witness(1, 32, Variant::Bech32m),
        ] {
            let parsed = to.parse::<Address<NetworkUnchecked>>();
            assert!(
                parsed.is_ok_and(|a| a.is_valid_for_network(Network::Testnet)),
                "{to}"
            );
            assert_eq!(payee(&to).err(), Some(CoreError::BadAddress), "{to}");
        }
        let script = ScriptHash::from_byte_array([3; 20]);
        let p2sh = Address::new(Network::Testnet, Payload::ScriptHash(script)).to_string();
        assert!(payee(&p2sh).is_ok(), "{p2sh}");
    }

    /// R-4: the limit is Dash Core's, by the kind of output: 546 duffs for P2PKH, 540 for P2SH.
    #[test]
    fn a_payment_below_the_dust_limit_is_too_small() {
        use key_wallet::dashcore::address::Payload;
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::dashcore::{Network, PubkeyHash, ScriptHash};
        let p2pkh = Address::new(
            Network::Testnet,
            Payload::PubkeyHash(PubkeyHash::from_byte_array([2; 20])),
        );
        let p2sh = Address::new(
            Network::Testnet,
            Payload::ScriptHash(ScriptHash::from_byte_array([3; 20])),
        );
        for (to, limit) in [(Some(&p2pkh), 546), (Some(&p2sh), 540), (None, 546)] {
            assert!(is_dust(to, limit - 1), "{to:?} below");
            assert!(!is_dust(to, limit), "{to:?} at the limit");
        }
    }

    #[test]
    fn the_fee_grows_with_the_coins_spent() {
        let one = fees(1).core_payment;
        assert!((200..400).contains(&one), "one coin: {one} duffs");
        assert!(fees(10).core_payment > one);
        assert_eq!(fees(0).core_payment, one);
    }

    /// BUG-55: a small payment from a wallet of many coins costs one coin's fee, "Max" what
    /// spending them all costs — not the same figure for both.
    #[test]
    fn a_payment_costs_the_coins_it_takes_not_every_coin() {
        let values = vec![5_000_000, 391_000_000, 100_000, 200_000];
        let mut f = fees(values.len());
        f.core_coins = core_coins(values);
        let largest: Vec<_> = f.core_coins.iter().map(|c| c.value).collect();
        assert_eq!(largest, [391_000_000, 5_000_000, 200_000, 100_000]);
        assert_eq!(f.core_coins[0].fee, fees(1).core_payment);
        assert_eq!(
            f.core_coins[3].fee, f.core_payment,
            "every coin: core_payment"
        );
        assert_eq!(f.core_payment_for(100_000), fees(1).core_payment);
        assert_eq!(
            f.core_payment_for(0),
            fees(1).core_payment,
            "nothing typed: one coin"
        );
        let all = 391_000_000 + 5_000_000 + 200_000 + 100_000 - f.core_payment;
        assert_eq!(f.core_payment_for(all), f.core_payment);
        // Just over the largest coin: two.
        assert_eq!(f.core_payment_for(391_000_000), fees(2).core_payment);
        assert!(fees(2).core_payment > fees(1).core_payment);
    }

    #[test]
    fn maps_sync_stages() {
        let mut s = SyncStatus::default();
        assert_eq!(sync_info(&s, false).stage, SyncStage::Connecting);
        s.phase = Phase::Syncing;
        s.headers = Some(Stage {
            height: 500,
            target: 1000,
            done: false,
        });
        let info = sync_info(&s, false);
        assert_eq!(info.stage, SyncStage::Headers);
        assert_eq!(info.progress, Some(0.5));
        s.headers = Some(Stage {
            height: 1000,
            target: 1000,
            done: true,
        });
        s.masternodes = Some(Stage {
            height: 900,
            target: 1000,
            done: true,
        });
        s.phase = Phase::Synced;
        assert_eq!(sync_info(&s, false).stage, SyncStage::ChainLock);
        s.chainlock_height = Some(995);
        let info = sync_info(&s, false);
        assert_eq!(info.stage, SyncStage::Done);
        assert!(info.chainlocked);
    }

    #[test]
    fn a_chainlock_far_below_the_tip_does_not_count() {
        let s = SyncStatus {
            phase: Phase::Synced,
            headers: Some(Stage {
                height: 1000,
                target: 1000,
                done: true,
            }),
            masternodes: Some(Stage {
                height: 1000,
                target: 1000,
                done: true,
            }),
            chainlock_height: Some(500),
            ..Default::default()
        };
        assert!(!sync_info(&s, false).chainlocked);
    }

    fn at_tip() -> SyncStatus {
        let tip = Stage {
            height: 1000,
            target: 1000,
            done: true,
        };
        SyncStatus {
            phase: Phase::Synced,
            headers: Some(tip),
            masternodes: Some(tip),
            chainlock_height: Some(1000),
            ..Default::default()
        }
    }

    /// BUG-162 AC-T1: the client's filters at the tip say nothing of the wallet — its own synced
    /// height does: 0 is «Filters», not «Done»; at the tip, «Done».
    #[test]
    fn the_wallets_own_height_says_whether_its_coins_are_known() {
        let mut s = at_tip();
        filters_done_at(&mut s, 1000);
        s.note_wallet_synced(Some(0));
        assert!(!coins_known(&s, true));
        assert_eq!(sync_info(&s, true).stage, SyncStage::Filters);
        s.note_wallet_synced(Some(1000));
        assert!(coins_known(&s, true));
        assert_eq!(sync_info(&s, true).stage, SyncStage::Done);
        // A new block: the headers one ahead, the wallet a few seconds behind — no flicker.
        s.headers = Some(Stage {
            height: 1001,
            target: 1001,
            done: true,
        });
        s.latch_wallet_at_tip();
        assert!(coins_known(&s, true), "kept through a new block");
        // Its pass restarted (dash-spv: synced_height fell below committed): not known.
        s.note_wallet_synced(Some(0));
        assert!(!coins_known(&s, true));
    }

    /// BUG-162 AC-T2, AC-T4: the customer's first start — the client's filters reach the tip
    /// with no wallet; a wallet joins; its own pass runs from 0 to the tip. «Filters» shows once,
    /// without a «Done» in between, and no count is checked until its pass is at the tip.
    #[test]
    fn a_first_start_shows_the_filters_once_for_the_wallet() {
        let mut s = at_tip();
        // Before any wallet: the filters run for nobody and reach the tip. No wallet: no
        // filters stage at all.
        filters_done_at(&mut s, 1000);
        assert_eq!(sync_info(&s, false).stage, SyncStage::Done);
        // The wallet joins the running client.
        s.wallet_attached();
        let mut stages = vec![sync_info(&s, true).stage];
        let before = s.clone();
        s.note_wallet_synced(Some(0));
        stages.push(sync_info(&s, true).stage);
        // dash-spv restarts the scan for it: committed at 0, then up to the tip.
        for height in [0, 400, 800, 999] {
            s.filters = Some(Stage {
                height,
                target: 1000,
                done: false,
            });
            s.latch_filters_at_tip();
            s.note_wallet_synced(Some(height));
            stages.push(sync_info(&s, true).stage);
            assert!(!count_checked(true, &before, &s), "not checked at {height}");
        }
        filters_done_at(&mut s, 1000);
        s.note_wallet_synced(Some(1000));
        stages.push(sync_info(&s, true).stage);
        let filters = stages
            .iter()
            .filter(|&&st| st == SyncStage::Filters)
            .count();
        assert_eq!(filters, stages.len() - 1, "{stages:?}");
        assert_eq!(*stages.last().unwrap(), SyncStage::Done);
        assert!(count_checked(true, &s.clone(), &s));
    }

    #[test]
    fn a_failure_before_any_header_is_still_connecting() {
        let s = SyncStatus {
            phase: Phase::Error,
            error: Some("no peers".into()),
            ..Default::default()
        };
        let info = sync_info(&s, false);
        assert_eq!(info.stage, SyncStage::Connecting);
        assert_eq!(info.error.as_deref(), Some("no peers"));
    }

    #[test]
    fn an_open_wallet_is_synced_only_with_its_filters() {
        let mut s = at_tip();
        assert_eq!(sync_info(&s, false).stage, SyncStage::Done);
        assert!(coins_known(&s, false));
        // Right after unlocking: no filter report yet.
        assert_eq!(sync_info(&s, true).stage, SyncStage::Filters);
        assert!(!coins_known(&s, true));
        s.filters = Some(Stage {
            height: 400,
            target: 1000,
            done: false,
        });
        let info = sync_info(&s, true);
        assert_eq!(info.stage, SyncStage::Filters);
        assert_eq!(info.filters_height, 400);
        assert!(!coins_known(&s, true));
        s.filters = Some(Stage {
            height: 1000,
            target: 1000,
            done: true,
        });
        s.latch_filters_at_tip();
        assert_eq!(sync_info(&s, true).stage, SyncStage::Done);
        assert!(coins_known(&s, true));
    }

    fn filters_done_at(s: &mut SyncStatus, height: u32) {
        s.filters = Some(Stage {
            height,
            target: height,
            done: true,
        });
        s.latch_filters_at_tip();
    }

    /// BUG-77 workarounds r1 should 2 (the race in `Real::wallet()`): the coins are counted
    /// while a rewound pass is below the tip, and the pass reaches it before the count is
    /// checked. Known only after: not checked — that count is the rewound pass's 4,82.
    #[test]
    fn a_count_is_checked_only_when_the_coins_were_known_before_and_after_it() {
        let mut before = at_tip();
        filters_done_at(&mut before, 1000);
        let known = before.clone();
        filters_done_at(&mut before, 900);
        assert!(!coins_known(&before, true), "a rewound pass below the tip");
        let mut after = before.clone();
        filters_done_at(&mut after, 1000);
        assert!(coins_known(&after, true), "the pass has reached the tip");
        assert!(
            !count_checked(true, &before, &after),
            "counted while the pass was below the tip"
        );
        assert!(!count_checked(true, &known, &before), "a rewind meanwhile");
        assert!(count_checked(true, &known, &after));
        assert!(!count_checked(false, &known, &after), "no balance counted");
    }

    /// BUG-77 r1 must: right after a start dash-spv says the filters are done — against the
    /// filter headers it has, i.e. the last run's height — while the headers are already at the
    /// network's tip. The coins are then the last run's: not known, so not shown as checked now
    /// and no basis for a payment. Known once the filters reach the headers' tip; a new block
    /// (headers one ahead for a few seconds) keeps them known; a rewind does not.
    #[test]
    fn filters_done_below_the_headers_tip_are_not_known_coins() {
        let mut s = at_tip();
        filters_done_at(&mut s, 990);
        assert!(
            !coins_known(&s, true),
            "headers at 1000, filters done at 990"
        );
        assert_eq!(
            sync_info(&s, true).stage,
            SyncStage::Filters,
            "not «Done» either"
        );
        filters_done_at(&mut s, 1000);
        assert!(coins_known(&s, true));
        assert_eq!(sync_info(&s, true).stage, SyncStage::Done);
        // A new block: the headers first.
        s.headers = Some(Stage {
            height: 1001,
            target: 1001,
            done: true,
        });
        s.latch_filters_at_tip();
        assert!(coins_known(&s, true), "no flicker on a new block");
        filters_done_at(&mut s, 1001);
        assert!(coins_known(&s, true));
        // Then dash-spv puts the filters back to syncing for a moment, as on every block (r2
        // should 1: the tile dimmed and the stage said «Filters» at each one).
        s.headers = Some(Stage {
            height: 1002,
            target: 1002,
            done: true,
        });
        s.filters = Some(Stage {
            height: 1001,
            target: 1002,
            done: false,
        });
        s.latch_filters_at_tip();
        assert!(
            coins_known(&s, true),
            "no flicker while the new filters come"
        );
        assert_eq!(sync_info(&s, true).stage, SyncStage::Done);
        filters_done_at(&mut s, 1002);
        // A rewind (DashPay's backfill, BUG-64's rescan): the filters below where they were.
        filters_done_at(&mut s, 900);
        assert!(!coins_known(&s, true));
        // A new client on the same status: the next run proves it again.
        filters_done_at(&mut s, 1002);
        assert!(coins_known(&s, true));
        s.begin_run();
        assert!(!coins_known(&s, true));
    }

    /// r2 should 2: the latch goes with a rewind. Meanwhile the headers went on two blocks; the
    /// filters climbing back to the old latch are not at the tip, and the two blocks above it
    /// were not scanned yet.
    #[test]
    fn after_a_rewind_the_filters_have_to_reach_the_new_tip() {
        let mut s = at_tip();
        filters_done_at(&mut s, 1000);
        assert!(coins_known(&s, true));
        filters_done_at(&mut s, 900);
        s.headers = Some(Stage {
            height: 1002,
            target: 1002,
            done: true,
        });
        filters_done_at(&mut s, 1000);
        assert!(
            !coins_known(&s, true),
            "the old latch at 1000, the tip at 1002"
        );
        assert_eq!(sync_info(&s, true).stage, SyncStage::Filters);
        filters_done_at(&mut s, 1002);
        assert!(coins_known(&s, true));
    }

    /// All peers lost: dash-spv's headers wait for connections again. Back, the headers reach a
    /// tip the filters have not: not known until the filters reach it, though they are past the
    /// latch of before.
    #[test]
    fn with_the_peers_lost_the_latch_goes() {
        let mut s = at_tip();
        filters_done_at(&mut s, 1000);
        s.headers = Some(Stage {
            height: 1000,
            target: 1000,
            done: false,
        });
        s.latch_filters_at_tip();
        assert!(!coins_known(&s, true));
        s.headers = Some(Stage {
            height: 1005,
            target: 1005,
            done: true,
        });
        s.filters = Some(Stage {
            height: 1001,
            target: 1005,
            done: false,
        });
        s.latch_filters_at_tip();
        assert!(!coins_known(&s, true), "filters at 1001 of 1005");
        filters_done_at(&mut s, 1005);
        assert!(coins_known(&s, true));
    }

    #[test]
    fn filters_move_while_a_batch_waits_for_its_blocks() {
        // As on testnet: batches done up to 447 000 of 1 583 991 (a restored wallet's blocks
        // are coming), filters already stored up to 1 400 000.
        let mut s = at_tip();
        s.filters = Some(Stage {
            height: 447_000,
            target: 1_583_991,
            done: false,
        });
        s.filters_stored = 447_000;
        // Restored with birth 0: this run began at the first batch.
        s.filters_from = Some(1);
        let waiting = sync_info(&s, true).progress.unwrap();
        s.filters_stored = 1_400_000;
        let info = sync_info(&s, true);
        assert_eq!(info.stage, SyncStage::Filters);
        // 28 % before, 68 % now: two thirds of the stage are the download, and it moved.
        let now = info.progress.unwrap();
        assert!(now > waiting + 0.25, "{now} vs {waiting}");
        assert!(now < 1.0, "not done until the batches are");
        // What is done stays what the history shows.
        assert_eq!(info.filters_height, 447_000);
    }

    /// The journal of 2026-11-06: a warm folder, 637 headers and 662 filters behind. The
    /// percent is of this run's work, not of the height (BUG-60).
    fn warm_start() -> SyncStatus {
        let mut s = SyncStatus {
            phase: Phase::Syncing,
            headers: Some(Stage {
                height: 1_585_857,
                target: 1_586_494,
                done: false,
            }),
            ..Default::default()
        };
        s.headers_from = Some(1_585_857);
        s
    }

    #[test]
    fn headers_count_from_where_this_run_began() {
        let mut s = warm_start();
        assert_eq!(sync_info(&s, true).progress, Some(0.0));
        // Segments downloaded side by side: the tip has not moved yet, they count.
        s.headers_buffered = 300;
        let info = sync_info(&s, true);
        assert_eq!(info.stage, SyncStage::Headers);
        let part = info.progress.unwrap();
        assert!((part - 300.0 / 637.0).abs() < 1e-3, "{part}");
        // The headers a node sent are all joined up.
        s.headers_buffered = 0;
        s.headers = Some(Stage {
            height: 1_586_494,
            target: 1_586_494,
            done: false,
        });
        assert_eq!(sync_info(&s, true).progress, Some(1.0));
        // At the tip from the start, the nodes not yet heard: no count, not «100 %».
        s.headers_from = Some(1_586_494);
        let info = sync_info(&s, true);
        assert_eq!(info.stage, SyncStage::Headers);
        assert_eq!(info.progress, None);
    }

    #[test]
    fn masternode_lists_are_a_step_without_a_count() {
        let mut s = warm_start();
        s.headers = Some(Stage {
            height: 1_586_494,
            target: 1_586_494,
            done: true,
        });
        // The saved lists, near the tip: height / tip is 99.9 %, and they are not done.
        s.masternodes = Some(Stage {
            height: 1_585_800,
            target: 1_586_494,
            done: false,
        });
        let info = sync_info(&s, true);
        assert_eq!(info.stage, SyncStage::Masternodes);
        assert_eq!(info.progress, None);
        // A node answered: its quorums are checked (BUG-61).
        s.lists_answered = true;
        let info = sync_info(&s, true);
        assert_eq!(info.stage, SyncStage::Quorums);
        assert_eq!(info.progress, None);
        // The lists done, the filters 662 behind: they count from where they were.
        s.masternodes = Some(Stage {
            height: 1_586_494,
            target: 1_586_494,
            done: true,
        });
        s.filters = Some(Stage {
            height: 1_585_832,
            target: 1_586_494,
            done: false,
        });
        s.filters_stored = 1_585_832;
        s.filter_headers = 1_586_494;
        s.filters_from = Some(1_585_832);
        let info = sync_info(&s, true);
        assert_eq!(info.stage, SyncStage::Filters);
        let part = info.progress.unwrap();
        assert!(
            (part - 1.0 / 3.0).abs() < 1e-3,
            "the filter headers only: {part}"
        );
    }

    #[test]
    fn a_stage_with_nothing_to_count_has_no_percent() {
        assert_eq!(sync_info(&SyncStatus::default(), false).progress, None);
        let mut s = at_tip();
        s.chainlock_height = None;
        let info = sync_info(&s, false);
        assert_eq!(info.stage, SyncStage::ChainLock);
        assert_eq!(info.progress, None);
        assert_eq!(
            sync_info(&at_tip(), true).progress,
            None,
            "filters not heard yet"
        );
        assert_eq!(sync_info(&at_tip(), false).progress, Some(1.0));
    }
}

#[cfg(test)]
mod kind_tests {
    use super::*;
    use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};

    #[test]
    fn tells_asset_locks_unlocks_and_payees_apart() {
        let out = TransactionDirection::Outgoing;
        assert_eq!(
            kind_of(out, TransactionType::AssetLock, None),
            (
                ActivityKind::TopUp {
                    identity: String::new(),
                    creates: false,
                    credits: evolite_api::Credits::Unknown
                },
                Chain::CoreToPlatform
            )
        );
        assert!(matches!(
            kind_of(
                TransactionDirection::Incoming,
                TransactionType::AssetUnlock,
                None
            ),
            (ActivityKind::Received { from: Some(_) }, Chain::Core)
        ));
        assert_eq!(
            kind_of(out, TransactionType::Standard, Some("yX".into())).0,
            ActivityKind::Sent { to: "yX".into() }
        );
        assert_eq!(
            kind_of(
                TransactionDirection::Internal,
                TransactionType::Standard,
                None
            )
            .0,
            ActivityKind::SelfTransfer { moved: 0 }
        );
    }

    /// BUG-33: a payment to one of our own receiving addresses names that address and what
    /// came there, so "Receive" lists it with the others.
    #[test]
    fn a_transfer_to_our_own_address_names_it_and_what_came() {
        use key_wallet::dashcore::address::Payload;
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::dashcore::{PubkeyHash, Transaction, TxIn};
        use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};

        let address = |n: u8| {
            Address::new(
                key_wallet::Network::Testnet,
                Payload::PubkeyHash(PubkeyHash::from_byte_array([n; 20])),
            )
        };
        let (ours, change) = (address(7), address(8));
        let detail = |index, role, at: &Address, value| OutputDetail {
            index,
            role,
            address: Some(at.clone()),
            value,
        };
        // Our coin of 0,00998 in: what came, the change and the fee (key-wallet has no transfer
        // of ours without an input of ours, BUG-85 r1 should 1).
        let mut record = TransactionRecord::new(
            Transaction {
                version: 2,
                lock_time: 0,
                input: vec![TxIn::default()],
                output: Vec::new(),
                special_transaction_payload: None,
            },
            key_wallet::account::AccountType::Standard {
                index: 0,
                standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
            },
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Internal,
            vec![InputDetail {
                index: 0,
                value: 998_000,
                address: change.clone(),
            }],
            vec![
                detail(0, OutputRole::Received, &ours, 100_000),
                detail(1, OutputRole::Change, &change, 897_774),
            ],
            -226,
        );
        record.fee = Some(226);
        let line = activity(&record, 0, true);
        assert_eq!(line.kind, ActivityKind::SelfTransfer { moved: 100_000 });
        assert_eq!(
            line.address,
            Some(ours.to_string()),
            "not the change address"
        );
        assert_eq!((line.amount, line.fee), (-226, Some(226)));
    }

    /// BUG-85, werner555's transfer of «All» to its own address: a contact's coin (0,0515) that
    /// account 0 also holds (BUG-77 U1) is spent with the rest; only the fee 0,00003778 left.
    /// Account 0's view: both coins in, all back; the contact account's: its coin in, nothing
    /// back. (A) The store's fold adds the two nets: −0,05153778. (B) This run's records, the
    /// contact account's first: it was picked alone — the coin out, nothing back.
    #[test]
    fn a_coin_two_accounts_hold_is_spent_once_in_the_history() {
        use key_wallet::dashcore::address::Payload;
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::dashcore::{OutPoint, PubkeyHash, Transaction, TxIn, TxOut, Txid};
        use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};

        const OWN: u64 = 381_424_464;
        const CONTACT: u64 = 5_150_000;
        const FEE: u64 = 3_778;
        let address = |n: u8| {
            Address::new(
                key_wallet::Network::Testnet,
                Payload::PubkeyHash(PubkeyHash::from_byte_array([n; 20])),
            )
        };
        let (mine, theirs) = (address(7), address(9));
        let back = OWN + CONTACT - FEE;
        let tx = Transaction {
            version: 2,
            lock_time: 0,
            input: [1u8, 2]
                .iter()
                .map(|n| TxIn {
                    previous_output: OutPoint::new(Txid::from_byte_array([*n; 32]), 0),
                    ..TxIn::default()
                })
                .collect(),
            output: vec![TxOut {
                value: back,
                script_pubkey: mine.script_pubkey(),
            }],
            special_transaction_payload: None,
        };
        let input = |index, value, at: &Address| InputDetail {
            index,
            value,
            address: at.clone(),
        };
        let output = |role| OutputDetail {
            index: 0,
            role,
            address: Some(mine.clone()),
            value: back,
        };
        let account = |index| key_wallet::account::AccountType::Standard {
            index,
            standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
        };
        let mut zero = TransactionRecord::new(
            tx.clone(),
            account(0),
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Internal,
            vec![input(0, OWN, &mine), input(1, CONTACT, &theirs)],
            vec![output(OutputRole::Received)],
            -(FEE as i64),
        );
        zero.fee = Some(FEE);
        let mut contact = TransactionRecord::new(
            tx,
            account(1),
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Outgoing,
            vec![input(1, CONTACT, &theirs)],
            vec![output(OutputRole::Sent)],
            -(CONTACT as i64),
        );
        contact.fee = Some(FEE);
        let fee_only = |line: Activity| {
            assert_eq!(
                line.kind,
                ActivityKind::SelfTransfer {
                    moved: back as Duffs
                }
            );
            assert_eq!(
                (line.amount, line.fee),
                (-(FEE as Duffs), Some(FEE as Duffs))
            );
        };
        // (A) platform-wallet's fold: details once by index, the nets added.
        let mut folded = zero.clone();
        folded.net_amount = -((CONTACT + FEE) as i64);
        assert_eq!(folded.net_amount, -5_153_778);
        fee_only(activity(&folded, 0, true));
        // (B) this run's records, the contact account's first.
        let one = one_per_transaction([&contact, &zero]);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].direction, TransactionDirection::Internal);
        fee_only(activity(&one[0], 0, true));
        // A payment out of the contact's coin alone stays what it was.
        let alone = one_per_transaction([&contact]);
        assert_eq!(activity(&alone[0], 0, true).amount, -(CONTACT as Duffs));
    }

    fn two_account_records(
        kind: TransactionType,
        outputs: usize,
        first: (Vec<InputDetail>, Vec<OutputDetail>, i64),
        second: (Vec<InputDetail>, Vec<OutputDetail>, i64),
    ) -> (TransactionRecord, TransactionRecord) {
        use key_wallet::dashcore::{Transaction, TxIn, TxOut};
        let tx = Transaction {
            version: 3,
            lock_time: 0,
            input: vec![TxIn::default()],
            output: vec![TxOut::default(); outputs],
            special_transaction_payload: None,
        };
        let record = |index, (ins, outs, net): (Vec<InputDetail>, Vec<OutputDetail>, i64)| {
            let direction = if ins.is_empty() {
                TransactionDirection::Incoming
            } else {
                TransactionDirection::Outgoing
            };
            TransactionRecord::new(
                tx.clone(),
                key_wallet::account::AccountType::Standard {
                    index,
                    standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
                },
                TransactionContext::Mempool,
                kind,
                direction,
                ins,
                outs,
                net,
            )
        };
        (record(0, first), record(1, second))
    }

    fn detail(index: u32, role: OutputRole, value: u64) -> OutputDetail {
        use key_wallet::dashcore::PubkeyHash;
        use key_wallet::dashcore::address::Payload;
        use key_wallet::dashcore::hashes::Hash;
        OutputDetail {
            index,
            role,
            address: Some(Address::new(
                key_wallet::Network::Testnet,
                Payload::PubkeyHash(PubkeyHash::from_byte_array([index as u8 + 1; 20])),
            )),
            value,
        }
    }

    /// BUG-85 r1 should 1: one payment in to two of our accounts (an address of account 0 and
    /// a contact's) is one line of both outputs, in this run as after a restart. The first
    /// account's net alone left the second output out of the sum.
    #[test]
    fn a_payment_in_to_two_of_our_accounts_counts_both() {
        let (zero, contact) = two_account_records(
            TransactionType::Standard,
            3,
            (
                Vec::new(),
                vec![detail(0, OutputRole::Received, 70_000)],
                70_000,
            ),
            (
                Vec::new(),
                vec![detail(2, OutputRole::Received, 30_000)],
                30_000,
            ),
        );
        let one = one_per_transaction([&zero, &contact]);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].direction, TransactionDirection::Incoming);
        assert_eq!(activity(&one[0], 0, true).amount, 100_000);
        // The store's fold of the same two (details by index, nets added) says the same.
        let mut folded = one[0].clone();
        folded.net_amount = 100_000;
        assert_eq!(activity(&folded, 0, true).amount, 100_000);
        // An output both accounts hold (the mirror of BUG-77 U1) is counted once, though the
        // fold adds it twice.
        let mut twice = zero.clone();
        twice.net_amount = 140_000;
        assert_eq!(activity(&twice, 0, true).amount, 70_000);
    }

    /// BUG-85 r1 should 2: an asset lock's records stay the spending account's alone. The other
    /// account's details are of the lock's payload, by its own indexes: folded by index with the
    /// transaction's outputs, the credit output would turn the burnt one into a coin received.
    #[test]
    fn an_asset_lock_is_left_unfolded() {
        const LOCKED: u64 = 1_000_000;
        const CHANGE: u64 = 500_000;
        const FEE: u64 = 263;
        let spending = (
            vec![InputDetail {
                index: 0,
                value: LOCKED + CHANGE + FEE,
                address: detail(5, OutputRole::Change, 0).address.unwrap(),
            }],
            vec![
                detail(0, OutputRole::Unspendable, LOCKED),
                detail(1, OutputRole::Change, CHANGE),
            ],
            -((LOCKED + FEE) as i64),
        );
        let payload = (
            Vec::new(),
            vec![detail(0, OutputRole::Received, LOCKED)],
            LOCKED as i64,
        );
        let (mut lock, top_up) =
            two_account_records(TransactionType::AssetLock, 2, spending, payload);
        lock.fee = Some(FEE);
        for order in [[&top_up, &lock], [&lock, &top_up]] {
            let one = one_per_transaction(order);
            assert_eq!(one.len(), 1);
            assert_eq!(
                one[0].account_type, lock.account_type,
                "the spending account's"
            );
            assert_eq!(one[0].net_amount, lock.net_amount);
            let roles: Vec<_> = one[0].output_details.iter().map(|d| d.role).collect();
            assert_eq!(roles, [OutputRole::Unspendable, OutputRole::Change]);
            let line = activity(&one[0], 0, true);
            assert_eq!(
                (line.amount, line.fee),
                (-((LOCKED + FEE) as Duffs), Some(FEE as Duffs))
            );
        }
    }
}

/// FEAT-0005a, AC-T2/T3: the card's own addresses and its proof in words, from records as
/// key-wallet keeps them (no network).
#[cfg(test)]
mod detail_tests {
    use super::*;
    use key_wallet::dashcore::address::Payload;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{BlockHash, OutPoint, PubkeyHash, Transaction, TxIn, TxOut, Txid};
    use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};
    use key_wallet::transaction_checking::BlockInfo;

    fn address(n: u8) -> Address {
        Address::new(
            key_wallet::Network::Testnet,
            Payload::PubkeyHash(PubkeyHash::from_byte_array([n; 20])),
        )
    }

    fn input(index: u32, value: u64, at: u8) -> InputDetail {
        InputDetail {
            index,
            value,
            address: address(at),
        }
    }

    fn output(index: u32, role: OutputRole, value: u64, at: u8) -> OutputDetail {
        OutputDetail {
            index,
            role,
            address: Some(address(at)),
            value,
        }
    }

    /// A transaction spending `spends` (txid byte, vout) and paying `pays` (address byte, value).
    fn tx(spends: &[(u8, u32)], pays: &[(u8, u64)]) -> Transaction {
        Transaction {
            version: 2,
            lock_time: 0,
            input: spends
                .iter()
                .map(|(t, vout)| TxIn {
                    previous_output: OutPoint::new(Txid::from_byte_array([*t; 32]), *vout),
                    ..TxIn::default()
                })
                .collect(),
            output: pays
                .iter()
                .map(|(at, value)| TxOut {
                    value: *value,
                    script_pubkey: address(*at).script_pubkey(),
                })
                .collect(),
            special_transaction_payload: None,
        }
    }

    fn record(
        tx: Transaction,
        context: TransactionContext,
        direction: TransactionDirection,
        inputs: Vec<InputDetail>,
        outputs: Vec<OutputDetail>,
        net: i64,
    ) -> TransactionRecord {
        TransactionRecord::new(
            tx,
            key_wallet::account::AccountType::Standard {
                index: 0,
                standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
            },
            context,
            TransactionType::Standard,
            direction,
            inputs,
            outputs,
            net,
        )
    }

    /// FEAT-0046 R-5 (AC-T-T3): a standard payment going out or between own addresses may be
    /// broadcast again; one coming in, an asset lock and a mixing round may not. AC-T-T4: one
    /// whose coin another transaction of the wallet spends was replaced.
    #[test]
    fn what_may_be_broadcast_again() {
        use crate::spv::chain::rebroadcast::replaced;
        let at = |dir| {
            record(
                tx(&[(1, 0)], &[(7, 1_000)]),
                TransactionContext::Mempool,
                dir,
                vec![],
                vec![],
                -1_000,
            )
        };
        assert!(resendable(&at(TransactionDirection::Outgoing)));
        // Confirmed: not again (batch 63).
        let mut mined = at(TransactionDirection::Outgoing);
        mined.context =
            TransactionContext::InBlock(BlockInfo::new(100, BlockHash::all_zeros(), 1_700_000_000));
        assert!(!resendable(&mined));
        assert!(resendable(&at(TransactionDirection::Internal)));
        assert!(!resendable(&at(TransactionDirection::Incoming)));
        assert!(!resendable(&at(TransactionDirection::CoinJoin)));
        let mut lock = at(TransactionDirection::Outgoing);
        lock.transaction_type = TransactionType::AssetLock;
        assert!(!resendable(&lock));
        let mut mixing = at(TransactionDirection::Outgoing);
        mixing.transaction_type = TransactionType::CoinJoin;
        assert!(!resendable(&mixing));
        // Replaced: another spends its coin (1, 0); one spending another coin does not.
        let stuck = at(TransactionDirection::Outgoing);
        let other = record(
            tx(&[(1, 0), (2, 0)], &[(8, 900)]),
            TransactionContext::Mempool,
            TransactionDirection::Outgoing,
            vec![],
            vec![],
            -900,
        );
        let apart = record(
            tx(&[(3, 0)], &[(8, 900)]),
            TransactionContext::Mempool,
            TransactionDirection::Outgoing,
            vec![],
            vec![],
            -900,
        );
        assert!(!replaced(&stuck, &[&stuck, &apart]));
        assert!(replaced(&stuck, &[&stuck, &other, &apart]));
    }

    /// FEAT-0005 АРХ-2: an asset unlock gives the index Platform wrote into its withdrawal; any
    /// other transaction gives none.
    #[test]
    fn an_asset_unlock_gives_its_index() {
        use key_wallet::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
        use key_wallet::dashcore::blockdata::transaction::special_transaction::asset_unlock::{
            qualified_asset_unlock::AssetUnlockPayload, request_info::AssetUnlockRequestInfo,
            unqualified_asset_unlock::AssetUnlockBasePayload,
        };
        let mut unlock = record(
            tx(&[], &[(7, 100_000)]),
            TransactionContext::Mempool,
            TransactionDirection::Incoming,
            vec![],
            vec![output(0, OutputRole::Received, 100_000, 7)],
            100_000,
        );
        unlock.transaction_type = TransactionType::AssetUnlock;
        unlock.transaction.special_transaction_payload = Some(
            TransactionPayload::AssetUnlockPayloadType(AssetUnlockPayload {
                base: AssetUnlockBasePayload {
                    version: 1,
                    index: 4_321,
                    fee: 0,
                },
                request_info: AssetUnlockRequestInfo {
                    request_height: 1,
                    quorum_hash: key_wallet::dashcore::QuorumHash::all_zeros(),
                },
                quorum_sig: key_wallet::dashcore::bls_sig_utils::BLSSignature::from([0; 96]),
            }),
        );
        let plain = record(
            tx(&[(1, 0)], &[(8, 5)]),
            TransactionContext::Mempool,
            TransactionDirection::Outgoing,
            vec![input(0, 10, 5)],
            vec![],
            -5,
        );
        let found = asset_unlocks(&[unlock.clone(), plain]);
        assert_eq!(found.len(), 1);
        assert_eq!(found.get(&unlock.txid.to_string()), Some(&4_321));
    }

    fn detail(line: &Activity) -> &Detail {
        line.detail.as_deref().expect("a Core line has its detail")
    }

    /// One input of ours → `Received` → `Change`: from the input's address to the receiving
    /// one, the change not shown, the fee apart; ChainLock in words with its block.
    #[test]
    fn a_transfer_between_own_addresses_says_from_where_to_where() {
        let locked = TransactionContext::InChainLockedBlock(BlockInfo::new(
            1_585_795,
            BlockHash::all_zeros(),
            1_762_300_000,
        ));
        let mut r = record(
            tx(&[(1, 0)], &[(7, 100_000), (8, 897_774)]),
            locked,
            TransactionDirection::Internal,
            vec![input(0, 998_000, 5)],
            vec![
                output(0, OutputRole::Received, 100_000, 7),
                output(1, OutputRole::Change, 897_774, 8),
            ],
            -226,
        );
        r.fee = Some(226);
        let line = activity(&r, 0, true);
        assert_eq!(line.kind, ActivityKind::SelfTransfer { moved: 100_000 });
        assert_eq!((line.amount, line.fee), (-226, Some(226)));
        assert_eq!(
            detail(&line),
            &Detail {
                confirm: Some(Confirm::ChainLock { block: 1_585_795 }),
                own_from: vec![address(5).to_string()],
                own_to: vec![(address(7).to_string(), 100_000)],
                incomplete: false,
                ..Detail::default()
            }
        );
    }

    /// Several inputs: one address each, as a payment out counts them («3 inputs»).
    #[test]
    fn a_payment_out_lists_each_input_of_ours() {
        let r = record(
            tx(&[(1, 0), (2, 1), (3, 0)], &[(9, 10_000_000), (8, 5_000)]),
            TransactionContext::Mempool,
            TransactionDirection::Outgoing,
            vec![
                input(0, 4_000_000, 5),
                input(1, 4_000_000, 6),
                input(2, 2_005_226, 5),
            ],
            vec![
                output(0, OutputRole::Sent, 10_000_000, 9),
                output(1, OutputRole::Change, 5_000, 8),
            ],
            -10_000_226,
        );
        let line = activity(&r, 0, true);
        assert!(matches!(line.kind, ActivityKind::Sent { .. }));
        let d = detail(&line);
        assert_eq!(
            d.own_from,
            [5, 6, 5].map(|n| address(n).to_string()).to_vec()
        );
        assert!(d.own_to.is_empty(), "the change is not shown");
        assert_eq!(d.confirm, None, "in the mempool: the chip says it");
        assert!(!d.incomplete);
    }

    /// A spend recorded before the coin it spends (a restore): once `attribute_late_inputs` gives
    /// it the input back, the card has the same addresses as when it was recorded whole.
    #[test]
    fn a_spend_recorded_before_its_coin_gives_the_same_addresses() {
        let coin = record(
            tx(&[(1, 0)], &[(5, 998_000)]),
            TransactionContext::Mempool,
            TransactionDirection::Incoming,
            Vec::new(),
            vec![output(0, OutputRole::Received, 998_000, 5)],
            998_000,
        );
        let mut spend = tx(&[(0, 0)], &[(7, 100_000), (8, 897_774)]);
        spend.input[0].previous_output = OutPoint::new(coin.txid, 0);
        let outputs = vec![
            output(0, OutputRole::Received, 100_000, 7),
            output(1, OutputRole::Change, 897_774, 8),
        ];
        let whole = record(
            spend.clone(),
            TransactionContext::Mempool,
            TransactionDirection::Internal,
            vec![input(0, 998_000, 5)],
            outputs.clone(),
            -226,
        );
        let late = record(
            spend,
            TransactionContext::Mempool,
            TransactionDirection::Incoming,
            Vec::new(),
            outputs,
            997_774,
        );
        let mut records = vec![coin, late];
        attribute_late_inputs(&mut records, |_| None);
        let late = activity(&records[1], 0, true);
        let whole = activity(&whole, 0, true);
        assert_eq!(late.kind, whole.kind);
        assert_eq!(detail(&late), detail(&whole));
        assert!(!detail(&late).incomplete);
    }

    /// One account's view of a transfer to another of the wallet's accounts: its coin in, only
    /// change of its own back — where the coins went is not in the record (АРХ-3).
    #[test]
    fn a_transfer_the_record_cannot_follow_is_incomplete() {
        let r = record(
            tx(&[(1, 0)], &[(8, 897_774), (9, 100_000)]),
            TransactionContext::Mempool,
            TransactionDirection::Internal,
            vec![input(0, 998_000, 5)],
            vec![output(0, OutputRole::Change, 897_774, 8)],
            -100_226,
        );
        let line = activity(&r, 0, true);
        assert!(matches!(line.kind, ActivityKind::SelfTransfer { .. }));
        assert!(detail(&line).incomplete);
        // A payment in with none of our outputs listed is incomplete too.
        let lost = record(
            tx(&[(1, 0)], &[(9, 5)]),
            TransactionContext::Mempool,
            TransactionDirection::Incoming,
            Vec::new(),
            Vec::new(),
            5,
        );
        assert!(detail(&activity(&lost, 0, true)).incomplete);
    }

    /// Review r1 should 2 (D3): a payment out whose inputs of ours were not kept says so; one
    /// in with its output of ours does not.
    #[test]
    fn a_payment_out_without_its_inputs_is_incomplete() {
        let sent = record(
            tx(&[(1, 0)], &[(9, 10_000)]),
            TransactionContext::Mempool,
            TransactionDirection::Outgoing,
            Vec::new(),
            vec![output(0, OutputRole::Sent, 10_000, 9)],
            -10_226,
        );
        let line = activity(&sent, 0, true);
        assert!(
            matches!(line.kind, ActivityKind::Sent { .. }),
            "{:?}",
            line.kind
        );
        assert!(detail(&line).incomplete);
        let received = record(
            tx(&[(1, 0)], &[(5, 5_000)]),
            TransactionContext::Mempool,
            TransactionDirection::Incoming,
            Vec::new(),
            vec![output(0, OutputRole::Received, 5_000, 5)],
            5_000,
        );
        let line = activity(&received, 0, true);
        assert!(matches!(line.kind, ActivityKind::Received { .. }));
        assert!(!detail(&line).incomplete);
    }

    /// AC-T3: an InstantLock in words only when verified (BUG-26); a block not locked yet has
    /// none until a ChainLock over it settles it — then with its block.
    #[test]
    fn the_proof_in_words_follows_the_chip() {
        let r = |context| {
            record(
                tx(&[], &[(7, 5)]),
                context,
                TransactionDirection::Incoming,
                Vec::new(),
                vec![output(0, OutputRole::Received, 5, 7)],
                5,
            )
        };
        let is = || TransactionContext::InstantSend(Default::default());
        assert_eq!(
            detail(&activity(&r(is()), 0, true)).confirm,
            Some(Confirm::InstantLock)
        );
        assert_eq!(detail(&activity(&r(is()), 0, false)).confirm, None);
        let in_block = TransactionContext::InBlock(BlockInfo::new(
            1_000,
            BlockHash::all_zeros(),
            1_762_300_000,
        ));
        let mut history = vec![activity(&r(in_block), 0, true)];
        assert_eq!(detail(&history[0]).confirm, None);
        settle(&mut history, Some(999));
        assert_eq!(detail(&history[0]).confirm, None, "not under the lock");
        settle(&mut history, Some(1_001));
        assert_eq!(history[0].proof, Proof::ChainLock);
        assert_eq!(
            detail(&history[0]).confirm,
            Some(Confirm::ChainLock { block: 1_000 })
        );
    }

    /// BUG-86, werner555's «All» to its own address ydAH…WiXC (account 0, receiving), on a fresh
    /// copy: the rescan after the sync recorded the transaction in the contact account too, and
    /// that slice alone stayed in the store — the contact's coins in, the output `Sent` (the
    /// contact account does not know account 0's address). The late inputs then made it a
    /// payment of everything out: «Sent ydAH…WiXC −3,86574464».
    #[test]
    fn a_send_of_all_to_an_own_address_kept_as_another_accounts_slice_is_a_transfer() {
        const FEE: u64 = 3_778;
        let (own, contact) = ([381_000_000u64, 424_464], [5_000_000u64, 150_000]);
        // The coins, as stored: account 0's on address 5, the contact account's on address 6.
        let coin = |n: u8, value: u64, at: u8| {
            record(
                tx(&[(n, 0)], &[(at, value)]),
                TransactionContext::Mempool,
                TransactionDirection::Incoming,
                Vec::new(),
                vec![output(0, OutputRole::Received, value, at)],
                value as i64,
            )
        };
        let coins = [
            coin(1, own[0], 5),
            coin(2, own[1], 5),
            coin(3, contact[0], 6),
            coin(4, contact[1], 6),
        ];
        let all: u64 = own.iter().chain(&contact).sum();
        let back = all - FEE;
        let mut spend = tx(&[(0, 0); 4], &[(7, back)]);
        for (input, coin) in spend.input.iter_mut().zip(&coins) {
            input.previous_output = OutPoint::new(coin.txid, 0);
        }
        // The contact account's slice: its two coins (inputs 2 and 3), the output not its own.
        let slice = record(
            spend,
            TransactionContext::Mempool,
            TransactionDirection::Outgoing,
            vec![input(2, contact[0], 6), input(3, contact[1], 6)],
            vec![output(0, OutputRole::Sent, back, 7)],
            -((contact[0] + contact[1]) as i64),
        );
        // The wallet's own addresses: 5 and 7 of account 0, 6 of the contact account.
        let ours = |a: &Address| {
            [5, 6, 7]
                .iter()
                .any(|n| address(*n) == *a)
                .then_some(OutputRole::Received)
        };
        let shown = |slice: &TransactionRecord, ours: &dyn Fn(&Address) -> Option<OutputRole>| {
            let mut records: Vec<_> = coins.iter().cloned().chain([slice.clone()]).collect();
            for r in &mut records {
                own_outputs(r, ours);
            }
            attribute_late_inputs(&mut records, |_| None);
            activity(&records[4], 0, true)
        };
        let line = shown(&slice, &ours);
        assert_eq!(
            line.kind,
            ActivityKind::SelfTransfer {
                moved: back as Duffs
            }
        );
        assert_eq!(
            (line.amount, line.fee),
            (-(FEE as Duffs), Some(FEE as Duffs))
        );
        assert_eq!(line.address, Some(address(7).to_string()));
        let d = detail(&line);
        assert_eq!(d.own_from.len(), 4, "every coin of ours, once");
        assert_eq!(d.own_to, vec![(address(7).to_string(), back as Duffs)]);
        assert!(!d.incomplete);
        // The same slice paying an address that is not the wallet's stays a payment out.
        let theirs = |a: &Address| (address(7) != *a).then(|| ours(a)).flatten();
        let paid = shown(&slice, &theirs);
        assert!(matches!(&paid.kind, ActivityKind::Sent { to } if *to == address(7).to_string()));
        assert_eq!(paid.amount, -(all as Duffs));
        // A change address is change: the line is still a transfer, nothing "received" shown.
        let mut alone = slice.clone();
        assert!(own_outputs(&mut alone, |_| Some(OutputRole::Change)));
        assert_eq!(alone.output_details[0].role, OutputRole::Change);
        assert_eq!(alone.direction, TransactionDirection::Internal);
        // A record with nothing of ours as `Sent` is left as it is.
        let mut kept = coins[0].clone();
        assert!(!own_outputs(&mut kept, ours));
        assert_eq!(kept.net_amount, own[0] as i64);
        // An asset lock keeps its details: there they are of the lock's payload.
        let mut lock = slice.clone();
        lock.transaction_type = TransactionType::AssetLock;
        assert!(!own_outputs(&mut lock, ours));
        assert_eq!(lock.output_details[0].role, OutputRole::Sent);
    }

    /// BUG-194 AC-T1, AC-T2 (qa-f21's kestrel7): 0.0001 from @1108c to @1108b and back, both
    /// identities of this wallet. The store's pools end at index 29, so the change of each (3, 5)
    /// is `Sent` in the stored records and tx2 lists no input. By the store alone tx1 showed its
    /// change paid out (−0,1452581) and tx2 only «received +0,0001»; with the live wallet's own
    /// addresses both are transfers between own addresses, the fee only, and tx2 spends tx1's
    /// change.
    #[test]
    fn change_past_the_stored_pools_is_ours_by_the_live_wallet() {
        const FEE: u64 = 226;
        let coin = record(
            tx(&[(9, 0)], &[(1, 14_535_810)]),
            TransactionContext::Mempool,
            TransactionDirection::Incoming,
            Vec::new(),
            vec![output(0, OutputRole::Received, 14_535_810, 1)],
            14_535_810,
        );
        let change1 = 14_535_810 - 10_000 - FEE;
        let mut pay1 = tx(&[(0, 0)], &[(2, 10_000), (3, change1)]);
        pay1.input[0].previous_output = OutPoint::new(coin.txid, 0);
        // As stored: account 0 spent the coin; 2 is @1108b's receiving address (in the stored
        // pools), 3 the change past them.
        let tx1 = record(
            pay1,
            TransactionContext::Mempool,
            TransactionDirection::Outgoing,
            vec![input(0, 14_535_810, 1)],
            vec![
                output(0, OutputRole::Received, 10_000, 2),
                output(1, OutputRole::Sent, change1, 3),
            ],
            -(change1 as i64 + FEE as i64),
        );
        let change2 = change1 - 10_000 - FEE;
        let mut pay2 = tx(&[(0, 0)], &[(4, 10_000), (5, change2)]);
        pay2.input[0].previous_output = OutPoint::new(tx1.txid, 1);
        // As stored: @1108c's receiving account's slice — its output in, nothing spent.
        let tx2 = record(
            pay2,
            TransactionContext::Mempool,
            TransactionDirection::Incoming,
            Vec::new(),
            vec![
                output(0, OutputRole::Received, 10_000, 4),
                output(1, OutputRole::Sent, change2, 5),
            ],
            10_000,
        );
        let live = |a: &Address| {
            [3u8, 5]
                .iter()
                .any(|n| address(*n) == *a)
                .then_some(OutputRole::Change)
        };
        let shown = |ours: &dyn Fn(&Address) -> Option<OutputRole>| {
            let mut records = vec![coin.clone(), tx1.clone(), tx2.clone()];
            for r in &mut records {
                own_outputs(r, ours);
            }
            attribute_late_inputs(&mut records, |_| None);
            (
                activity(&records[1], 0, true),
                activity(&records[2], 0, true),
            )
        };
        let (one, two) = shown(&live);
        for line in [&one, &two] {
            assert!(
                matches!(line.kind, ActivityKind::SelfTransfer { .. }),
                "{:?}",
                line.kind
            );
            assert_eq!(line.amount, -(FEE as Duffs));
        }
        // Both pay a DashPay receiving address of one identity from the other: «Transfer
        // between your identities», with what moved; a contact outside the wallet is not one.
        let ids = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<std::collections::HashMap<_, _>>()
        };
        let ours = ids(&[("B", "@1108b"), ("C", "@1108c")]);
        let receipts = [(2u8, ("B", "C")), (4, ("C", "B"))]
            .into_iter()
            .map(|(n, (to, from))| (address(n).to_string(), (to.to_string(), from.to_string())))
            .collect();
        let mut history = vec![
            ("1".to_string(), one.clone()),
            ("2".to_string(), two.clone()),
        ];
        name_identity_transfers(&mut history, &receipts, &ours);
        for ((_, line), (from, to, at)) in history
            .iter()
            .zip([("@1108c", "@1108b", 2u8), ("@1108b", "@1108c", 4)])
        {
            assert_eq!(
                line.kind,
                ActivityKind::IdentityTransfer {
                    from: from.into(),
                    to: to.into(),
                    moved: 10_000
                }
            );
            assert_eq!(line.address, Some(address(at).to_string()));
            assert_eq!(line.amount, -(FEE as Duffs));
        }
        let mut history = vec![("1".to_string(), one.clone())];
        name_identity_transfers(&mut history, &receipts, &ids(&[("B", "@1108b")]));
        assert!(matches!(
            history[0].1.kind,
            ActivityKind::SelfTransfer { .. }
        ));
        // By the store's pools alone (the mutant): the change paid out, and a receipt.
        let (one, two) = shown(&|_| None);
        assert!(matches!(&one.kind, ActivityKind::Sent { to } if *to == address(3).to_string()));
        assert_eq!(one.amount, -((change1 + FEE) as Duffs));
        assert_eq!(two.amount, 10_000);
    }
}

/// AC-T8 measured, not asserted (timing depends on the machine): run with
/// `cargo test -p evolite-chain ac_t8 -- --ignored --nocapture`.
#[cfg(test)]
mod detail_cost {
    use super::*;
    use key_wallet::dashcore::address::Payload;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{OutPoint, PubkeyHash, Transaction, TxIn, TxOut, Txid};
    use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};
    use std::time::Instant;

    fn address(n: u32) -> Address {
        let mut b = [0u8; 20];
        b[..4].copy_from_slice(&n.to_le_bytes());
        Address::new(
            key_wallet::Network::Testnet,
            Payload::PubkeyHash(PubkeyHash::from_byte_array(b)),
        )
    }

    /// A payment out: `inputs` coins of ours, one payee, change back.
    fn send(n: u32, inputs: u32) -> TransactionRecord {
        let mut t = [0u8; 32];
        t[..4].copy_from_slice(&n.to_le_bytes());
        let tx = Transaction {
            version: 2,
            lock_time: 0,
            input: (0..inputs)
                .map(|i| TxIn {
                    previous_output: OutPoint::new(Txid::from_byte_array(t), i),
                    ..TxIn::default()
                })
                .collect(),
            output: vec![
                TxOut {
                    value: 1_000,
                    script_pubkey: address(n).script_pubkey(),
                },
                TxOut {
                    value: 500,
                    script_pubkey: address(n + 1).script_pubkey(),
                },
            ],
            special_transaction_payload: None,
        };
        TransactionRecord::new(
            tx,
            key_wallet::account::AccountType::Standard {
                index: 0,
                standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
            },
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Outgoing,
            (0..inputs)
                .map(|i| InputDetail {
                    index: i,
                    value: 1_000,
                    address: address(n + 10 + i),
                })
                .collect(),
            vec![
                OutputDetail {
                    index: 0,
                    role: OutputRole::Sent,
                    address: Some(address(n)),
                    value: 1_000,
                },
                OutputDetail {
                    index: 1,
                    role: OutputRole::Change,
                    address: Some(address(n + 1)),
                    value: 500,
                },
            ],
            -1_226,
        )
    }

    #[test]
    #[ignore = "a measurement: prints, asserts nothing"]
    fn ac_t8_what_the_detail_costs_per_snapshot() {
        const LINES: u32 = 2_000;
        const ROUNDS: u32 = 20;
        let records: Vec<_> = (0..LINES).map(|n| send(n * 100, 3)).collect();
        let started = Instant::now();
        for _ in 0..ROUNDS {
            let lines: Vec<_> = records.iter().map(|r| activity(r, 0, true)).collect();
            std::hint::black_box(lines);
        }
        let whole = started.elapsed() / ROUNDS;
        let kind = ActivityKind::Sent { to: String::new() };
        let started = Instant::now();
        for _ in 0..ROUNDS {
            let details: Vec<_> = records
                .iter()
                .map(|r| detail_of(r, &kind, Proof::Mempool, None))
                .collect();
            std::hint::black_box(details);
        }
        let detail = started.elapsed() / ROUNDS;
        println!(
            "AC-T8: {LINES} Core lines (3 inputs each): convert {whole:?}, of it Detail {detail:?} \
             ({:.1} µs a line)",
            detail.as_secs_f64() * 1e6 / f64::from(LINES)
        );
    }
}

/// FEAT-0030 phase A: the CoinJoin account's coins apart from Core.
#[cfg(test)]
mod coinjoin_tests {
    use super::*;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{OutPoint, Transaction, TxIn, TxOut, Txid};
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;

    const D: u64 = 100_001;

    fn info() -> ManagedWalletInfo {
        let keys = super::super::keys_of(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        ManagedWalletInfo::from_wallet(&keys, 0)
    }

    /// The `n`-th address of the BIP44 account 0 or of the CoinJoin account 0.
    fn address(info: &ManagedWalletInfo, coinjoin: bool, n: usize) -> Address {
        let accounts = info.accounts();
        let account = if coinjoin {
            &accounts.coinjoin_accounts[&0]
        } else {
            &accounts.standard_bip44_accounts[&0]
        };
        account.keys().managed_account_type().all_addresses()[n].clone()
    }

    fn other(n: u8) -> Address {
        Address::new(
            key_wallet::Network::Testnet,
            key_wallet::dashcore::address::Payload::PubkeyHash(
                key_wallet::dashcore::PubkeyHash::from_byte_array([n; 20]),
            ),
        )
    }

    fn foreign(n: u8) -> OutPoint {
        OutPoint::new(Txid::from_byte_array([n; 32]), 3)
    }

    fn utxo(outpoint: OutPoint, value: u64, at: &Address) -> key_wallet::Utxo {
        let mut u = key_wallet::Utxo::new(
            outpoint,
            TxOut {
                value,
                script_pubkey: at.script_pubkey(),
            },
            at.clone(),
            1_570_000,
            false,
        );
        u.is_confirmed = true;
        u
    }

    /// Made-up transactions of the wallet, kept for the records of the CoinJoin account.
    #[derive(Default)]
    struct Txs {
        made: Vec<Transaction>,
    }

    impl Txs {
        fn tx(&mut self, inputs: Vec<OutPoint>, outputs: Vec<(u64, Address)>) -> Txid {
            let tx = Transaction {
                version: 2,
                lock_time: self.made.len() as u32 + 1,
                input: inputs
                    .into_iter()
                    .map(|o| TxIn {
                        previous_output: o,
                        ..Default::default()
                    })
                    .collect(),
                output: outputs
                    .iter()
                    .map(|(value, at)| TxOut {
                        value: *value,
                        script_pubkey: at.script_pubkey(),
                    })
                    .collect(),
                special_transaction_payload: None,
            };
            let txid = tx.txid();
            self.made.push(tx);
            txid
        }

        /// One mixing round of `coin` with two strangers, ours to `to`.
        fn round(&mut self, coin: OutPoint, to: &Address, n: u8) -> OutPoint {
            let txid = self.tx(
                vec![foreign(n), coin, foreign(n + 1)],
                vec![(D, to.clone()), (D, other(n)), (D, other(n + 1))],
            );
            OutPoint::new(txid, 0)
        }
    }

    /// BUG-148: a contact's coin filed in account 0 by the store's early load goes to its own
    /// DashPay receiving account once that exists (the filters do not look at its block again):
    /// then it can be signed for and «Max» takes it; the balance does not change.
    #[test]
    fn a_filed_coin_goes_to_the_account_that_derives_it() {
        let mut info = info();
        let own = address(&info, false, 0);
        // A contact's receiving account (DIP-15): any xpub not account 0's serves the test.
        let keys = super::super::keys_of(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let contact = key_wallet::Account {
            parent_wallet_id: None,
            account_type: key_wallet::account::AccountType::DashpayReceivingFunds {
                index: 0,
                user_identity_id: [1; 32],
                friend_identity_id: [2; 32],
            },
            network: key_wallet::Network::Testnet,
            account_xpub: keys.accounts.coinjoin_accounts[&0].account_xpub,
            is_watch_only: false,
        };
        let managed = key_wallet::managed_account::ManagedCoreFundsAccount::from_account(&contact);
        let theirs = managed.managed_account_type().all_addresses()[0].clone();
        info.accounts_mut()
            .insert_funds_bearing_account(managed)
            .unwrap();
        let mine = utxo(
            OutPoint::new(Txid::from_byte_array([1; 32]), 0),
            5_000_000,
            &own,
        );
        let filed = utxo(
            OutPoint::new(Txid::from_byte_array([2; 32]), 0),
            100_000,
            &theirs,
        );
        {
            let account = info
                .accounts_mut()
                .standard_bip44_accounts
                .get_mut(&0)
                .unwrap();
            account.utxos.insert(mine.outpoint, mine.clone());
            account.utxos.insert(filed.outpoint, filed.clone());
        }
        // The journal's numbers by kind (BUG-150): BIP44, BIP32, DashPay, CoinJoin.
        assert_eq!(coin_kinds(&info)[0], (2, 5_100_000));
        assert_eq!(coin_kinds(&info)[2], (0, 0));
        assert_eq!(unsignable(&info), 100_000, "filed: not to sign for");
        assert_eq!(rehome_filed_coins(&mut info), 1);
        // The unlock's line after the move says so (BUG-148, QA pass 23: the line before it
        // showed the store's filing, and the coin seemed to move only after the filters).
        assert_eq!(
            rehomed_record(&info, 1),
            crate::diag::Record::Coins {
                when: "unlock_rehomed",
                bip44: (1, 5_000_000),
                bip32: (0, 0),
                dashpay: (1, 100_000),
                coinjoin: (0, 0),
                moved: Some(1),
            }
        );
        assert_eq!(coin_kinds(&info)[0], (1, 5_000_000));
        assert_eq!(coin_kinds(&info)[2], (1, 100_000));
        assert_eq!(unsignable(&info), 0, "in its own account: signed for");
        assert_eq!(core_balance(&info), 5_100_000, "the balance the same");
        let payable = payable_coins(&info, |_| true);
        assert!(payable.iter().any(|u| u.outpoint == filed.outpoint));
        assert_eq!(rehome_filed_coins(&mut info), 0, "nothing left to move");
    }

    /// BUG-142: a contact's coin the store's early load filed in account 0 (BUG-65), at an address
    /// that account does not derive, is the wallet's (the balance counts it) but no payment can
    /// sign for it: platform-wallet finds no derivation path, and «Max» from Core failed with
    /// «no derivation path for selected input address». A payment is offered only the coins
    /// whose account derives their address, and «Max» leaves the others out.
    #[test]
    fn a_coin_its_account_cannot_sign_for_is_not_payable() {
        let mut info = info();
        let own = address(&info, false, 0);
        let mine = utxo(
            OutPoint::new(Txid::from_byte_array([1; 32]), 0),
            5_000_000,
            &own,
        );
        let filed = utxo(
            OutPoint::new(Txid::from_byte_array([2; 32]), 0),
            100_000,
            &other(42),
        );
        {
            let account = info
                .accounts_mut()
                .standard_bip44_accounts
                .get_mut(&0)
                .unwrap();
            account.utxos.insert(mine.outpoint, mine.clone());
            account.utxos.insert(filed.outpoint, filed.clone());
        }
        assert_eq!(core_balance(&info), 5_100_000, "the wallet's all the same");
        let payable = payable_coins(&info, |_| true);
        assert_eq!(
            payable.iter().map(|u| u.outpoint).collect::<Vec<_>>(),
            vec![mine.outpoint],
            "only what can be signed for"
        );
        assert_eq!(unsignable(&info), 100_000);
    }

    /// AC-T2, BUG-121 p. 1: a coin of the CoinJoin account is no Core coin — not in the balance,
    /// not among the coins a payment may take; «Mixed» has it. Red when `coins_once` takes every
    /// funds account again.
    #[test]
    fn a_coinjoin_coin_is_not_core() {
        let mut info = info();
        let core_at = address(&info, false, 0);
        let mixed_at = address(&info, true, 0);
        let core = utxo(
            OutPoint::new(Txid::from_byte_array([1; 32]), 0),
            5_000_000,
            &core_at,
        );
        let mixed = utxo(
            OutPoint::new(Txid::from_byte_array([2; 32]), 0),
            D,
            &mixed_at,
        );
        {
            let accounts = info.accounts_mut();
            accounts
                .standard_bip44_accounts
                .get_mut(&0)
                .unwrap()
                .utxos
                .insert(core.outpoint, core.clone());
            accounts
                .coinjoin_accounts
                .get_mut(&0)
                .unwrap()
                .utxos
                .insert(mixed.outpoint, mixed.clone());
        }
        assert_eq!(core_balance(&info), 5_000_000);
        assert_eq!(coins_once(&info).len(), 1);
        let payable = payable_coins(&info, |_| true);
        assert!(payable.iter().all(|c| c.outpoint != mixed.outpoint));
        let coins = mixed_coins(&info, |_| true);
        assert_eq!(coins.len(), 1);
        assert_eq!(coins[0].utxo.outpoint, mixed.outpoint);
        // Not in the wallet's history: its rounds are not known, so it is not spendable.
        assert_eq!(
            mixed_balance(&coins),
            Some(Mixed {
                spendable: 0,
                unfinished: D as Duffs,
                rounds_needed: 4,
                max: 0,
            })
        );
        assert_eq!(mixed_balance(&[]), None, "nothing on the account: no tile");
    }

    /// AC-T3: of the CoinJoin account, only fully mixed final denominations are spendable; a coin
    /// of 3 rounds, and one of 5 whose InstantLock was not verified, are unfinished.
    #[test]
    fn only_fully_mixed_final_coins_are_spendable() {
        let mut info = info();
        let funding_at = address(&info, false, 0);
        let cj: Vec<Address> = (0..12).map(|n| address(&info, true, n)).collect();
        let mut txs = Txs::default();
        // Money in, the preparation (with change: 0 rounds), then rounds with two strangers.
        let money = txs.tx(vec![foreign(100)], vec![(5_000_000, funding_at.clone())]);
        let prep = txs.tx(
            vec![OutPoint::new(money, 0)],
            vec![
                (D, cj[0].clone()),
                (D, cj[1].clone()),
                (4_700_000, funding_at.clone()),
            ],
        );
        let mut four = OutPoint::new(prep, 0);
        for (i, n) in [(2, 10), (3, 20), (4, 30), (5, 40)] {
            four = txs.round(four, &cj[i], n);
        }
        let mut three = OutPoint::new(prep, 1);
        for (i, n) in [(6, 50), (7, 60), (8, 70)] {
            three = txs.round(three, &cj[i], n);
        }
        let unverified = txs.round(four, &cj[9], 80);
        {
            let account = info.accounts_mut().coinjoin_accounts.get_mut(&0).unwrap();
            for t in txs.made {
                let r = TransactionRecord::new(
                    t,
                    key_wallet::account::AccountType::CoinJoin { index: 0 },
                    TransactionContext::Mempool,
                    TransactionType::Standard,
                    TransactionDirection::Internal,
                    Vec::new(),
                    Vec::new(),
                    0,
                );
                account.transactions_mut().insert(r.txid, r);
            }
            account.utxos.insert(four, utxo(four, D, &cj[5]));
            account.utxos.insert(three, utxo(three, D, &cj[8]));
            let mut locked = utxo(unverified, D, &cj[9]);
            locked.is_confirmed = false;
            locked.is_instantlocked = true;
            account.utxos.insert(unverified, locked);
        }
        let coins = mixed_coins(&info, |_| false);
        let of = |o: OutPoint| coins.iter().find(|c| c.utxo.outpoint == o).unwrap();
        assert_eq!(of(four).rounds, evolite_coinjoin::Rounds::Mixed(4));
        assert!(of(four).spendable);
        assert_eq!(of(three).rounds, evolite_coinjoin::Rounds::Mixed(3));
        assert!(!of(three).spendable);
        assert_eq!(of(unverified).rounds, evolite_coinjoin::Rounds::Mixed(5));
        assert!(!of(unverified).spendable, "its InstantLock not verified");
        assert_eq!(
            mixed_balance(&coins),
            Some(Mixed {
                spendable: D as Duffs,
                unfinished: 2 * D as Duffs,
                rounds_needed: 4,
                max: D as Duffs - crate::mixed_pay::usual_fee(1) as Duffs,
            })
        );
    }

    /// AC-T8: the rounds of a wallet with 1 000 mixing transactions are counted in one pass, each
    /// outpoint once — within 50 ms in an optimised build. A debug build (the check's) is checked
    /// against a loose bound only, 2 s: it shows a pass gone quadratic, not the 50 ms.
    #[test]
    fn mixed_coins_are_counted_fast() {
        let mut info = info();
        let funding_at = address(&info, false, 0);
        let cj: Vec<Address> = (0..100).map(|n| address(&info, true, n)).collect();
        let mut txs = Txs::default();
        let mut finals = Vec::new();
        // 250 coins through 4 rounds each: 1 000 mixing transactions, and their preparations.
        for c in 0..250u32 {
            let n = (c % 200) as u8;
            let money = txs.tx(vec![foreign(n)], vec![(5_000_000, funding_at.clone())]);
            let prep = txs.tx(
                vec![OutPoint::new(money, 0)],
                vec![
                    (D, cj[(c % 100) as usize].clone()),
                    (4_800_000, funding_at.clone()),
                ],
            );
            let mut coin = OutPoint::new(prep, 0);
            for r in 0..4u32 {
                coin = txs.round(coin, &cj[((c + r + 1) % 100) as usize], n);
            }
            finals.push(coin);
        }
        {
            let account = info.accounts_mut().coinjoin_accounts.get_mut(&0).unwrap();
            for t in txs.made {
                let r = TransactionRecord::new(
                    t,
                    key_wallet::account::AccountType::CoinJoin { index: 0 },
                    TransactionContext::Mempool,
                    TransactionType::Standard,
                    TransactionDirection::Internal,
                    Vec::new(),
                    Vec::new(),
                    0,
                );
                account.transactions_mut().insert(r.txid, r);
            }
            for coin in &finals {
                account.utxos.insert(*coin, utxo(*coin, D, &cj[0]));
            }
        }
        let started = std::time::Instant::now();
        let coins = mixed_coins(&info, |_| true);
        let took = started.elapsed();
        assert_eq!(coins.iter().filter(|c| c.spendable).count(), 250);
        let bound = if cfg!(debug_assertions) { 2_000 } else { 50 };
        println!("AC-T8: 1 000 mixing transactions counted in {took:?} (bound {bound} ms)");
        assert!(took.as_millis() <= bound, "{took:?} over {bound} ms");
    }

    /// A history line of `tx` with these inputs of ours (input index, value, address) and outputs
    /// (index, role, address, value), as A3 shows it.
    fn line_of(
        info: &ManagedWalletInfo,
        tx: Transaction,
        ins: Vec<(u32, u64, Address)>,
        outs: Vec<(u32, OutputRole, Address, u64)>,
    ) -> Activity {
        use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};
        let spent: i64 = ins.iter().map(|(_, v, _)| *v as i64).sum();
        let got: i64 = outs
            .iter()
            .filter(|o| o.1 != OutputRole::Sent)
            .map(|o| o.3 as i64)
            .sum();
        let direction = if outs.iter().any(|o| o.1 == OutputRole::Sent) {
            TransactionDirection::Outgoing
        } else if ins.is_empty() {
            TransactionDirection::Incoming
        } else {
            TransactionDirection::Internal
        };
        let r = TransactionRecord::new(
            tx,
            key_wallet::account::AccountType::CoinJoin { index: 0 },
            TransactionContext::Mempool,
            TransactionType::Standard,
            direction,
            ins.into_iter()
                .map(|(index, value, address)| InputDetail {
                    index,
                    value,
                    address,
                })
                .collect(),
            outs.into_iter()
                .map(|(index, role, address, value)| OutputDetail {
                    index,
                    role,
                    address: Some(address),
                    value,
                })
                .collect(),
            got - spent,
        );
        let mut act = activity(&r, 0, false);
        mark_mixing(&mut act, &r, &coinjoin_addresses(info));
        act
    }

    /// AC-T9, A3: a round of two (key-wallet's mark misses it) and a taken collateral are
    /// «Mixing (CoinJoin)» in «Mixed» alone; the preparation is «Into mixing», Core to «Mixed»; a
    /// payment from CoinJoin coins stays «Sent», in «Mixed» instead of Core; an ordinary payment
    /// is not touched.
    #[test]
    fn mixing_lines_are_told_by_their_own_details() {
        use evolite_api::layer::{CORE, MIXED};
        let info = info();
        let core_at = address(&info, false, 0);
        let cj: Vec<Address> = (0..4).map(|n| address(&info, true, n)).collect();
        let mut txs = Txs::default();
        let tx = |txs: &mut Txs, ins: usize, outs: Vec<(u64, Address)>| {
            let inputs = (0..ins).map(|n| foreign(200 + n as u8)).collect();
            txs.tx(inputs, outs);
            txs.made.last().unwrap().clone()
        };

        let round = tx(&mut txs, 2, vec![(D, cj[1].clone()), (D, other(1))]);
        let act = line_of(
            &info,
            round,
            vec![(1, D, cj[0].clone())],
            vec![(0, OutputRole::Received, cj[1].clone(), D)],
        );
        assert_eq!(act.kind, ActivityKind::Mixing { fee: 0 });
        assert_eq!(
            (act.layers, act.flow, act.amount),
            (MIXED, Flow::Between, 0)
        );

        let collateral = tx(&mut txs, 1, vec![(10_000, cj[3].clone())]);
        let act = line_of(
            &info,
            collateral,
            vec![(0, 20_000, cj[2].clone())],
            vec![(0, OutputRole::Received, cj[3].clone(), 10_000)],
        );
        assert_eq!(act.kind, ActivityKind::Mixing { fee: 10_000 });
        assert_eq!(
            (act.layers, act.fee, act.amount),
            (MIXED, Some(10_000), -10_000)
        );

        let prep = tx(
            &mut txs,
            1,
            vec![
                (D, cj[0].clone()),
                (D, cj[1].clone()),
                (4_790_000, core_at.clone()),
            ],
        );
        let act = line_of(
            &info,
            prep,
            vec![(0, 5_000_000, core_at.clone())],
            vec![
                (0, OutputRole::Received, cj[0].clone(), D),
                (1, OutputRole::Received, cj[1].clone(), D),
                (2, OutputRole::Change, core_at.clone(), 4_790_000),
            ],
        );
        assert_eq!(act.kind, ActivityKind::IntoMixing);
        assert_eq!((act.layers, act.flow), (CORE | MIXED, Flow::Between));

        let pay = tx(&mut txs, 2, vec![(150_000, other(9))]);
        let act = line_of(
            &info,
            pay.clone(),
            vec![(0, D, cj[0].clone()), (1, D, cj[1].clone())],
            vec![(0, OutputRole::Sent, other(9), 150_000)],
        );
        assert!(
            matches!(act.kind, ActivityKind::Sent { .. }),
            "{:?}",
            act.kind
        );
        assert_eq!((act.layers, act.flow), (MIXED, Flow::Out));

        let act = line_of(
            &info,
            pay,
            vec![(0, 300_000, core_at.clone())],
            vec![(0, OutputRole::Sent, other(9), 150_000)],
        );
        assert!(
            matches!(act.kind, ActivityKind::Sent { .. }),
            "{:?}",
            act.kind
        );
        assert_eq!(act.layers, CORE, "an ordinary payment stays Core");
    }
}

/// The wallet's own Platform addresses as text (`tdash1…`): every address of its Platform
/// account, used or not — a transfer to one of them stays in the wallet (QA pass 21).
pub fn own_platform_addresses(
    info: &key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> std::collections::HashSet<String> {
    info.first_platform_payment_managed_account()
        .map(|account| {
            account
                .all_platform_addresses()
                .into_iter()
                .map(|a| {
                    dpp::address_funds::PlatformAddress::P2pkh(a.to_bytes())
                        .to_bech32m_string(crate::network::dash())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `to`, a recipient as it was typed, is one of `own` ([`own_platform_addresses`]).
/// bech32m may be typed in capitals (QA21 r1 P3 1): a valid address is never in mixed case and
/// its lowercase is the one spelling `own` holds; the network's prefix still has to match.
pub fn is_own_platform_address(own: &std::collections::HashSet<String>, to: &str) -> bool {
    let to = to.trim();
    dpp::address_funds::PlatformAddress::from_bech32m_string(to).is_ok()
        && own.contains(&to.to_ascii_lowercase())
}

#[cfg(test)]
mod own_address_tests {
    use super::is_own_platform_address;
    use dpp::address_funds::PlatformAddress;

    /// QA21 r1 P3 1: the wallet's address typed in capitals is still its own; mixed case is no
    /// address at all, and another address is another's.
    #[test]
    fn an_own_platform_address_is_known_in_capitals_too() {
        let text =
            |seed: u8| PlatformAddress::P2pkh([seed; 20]).to_bech32m_string(crate::network::dash());
        let own: std::collections::HashSet<String> = [text(7)].into();
        let mine = text(7);
        assert!(is_own_platform_address(&own, &mine));
        assert!(is_own_platform_address(&own, &mine.to_ascii_uppercase()));
        assert!(is_own_platform_address(&own, &format!("  {mine} ")));
        let mut mixed = mine.clone();
        mixed.replace_range(..1, &mine[..1].to_ascii_uppercase());
        assert!(!is_own_platform_address(&own, &mixed), "{mixed}");
        assert!(!is_own_platform_address(&own, &text(8)));
        assert!(!is_own_platform_address(&own, "not an address"));
    }
}

/// SPV item 3: the coins that are not final are frozen for platform-wallet's own builders, and
/// freed after.
#[cfg(test)]
mod freeze_tests {
    use super::*;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{OutPoint, TxOut, Txid};
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
    use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    use std::collections::HashMap;

    /// A confirmed coin, one InstantSend-locked, one marked InstantSend by a lock not verified
    /// (dash-spv's mark, or one the store restored, BUG-173), our own change waiting for its
    /// block, and one a payment holds (locked by its reservation).
    fn wallet() -> (ManagedWalletInfo, [OutPoint; 5]) {
        let keys = super::super::keys_of(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        let account = info
            .accounts_mut()
            .standard_bip44_accounts
            .get_mut(&0)
            .unwrap();
        let address = account.keys().managed_account_type().all_addresses()[0].clone();
        let mut points = [OutPoint::null(); 5];
        for (n, point) in points.iter_mut().enumerate() {
            *point = OutPoint::new(Txid::from_byte_array([n as u8 + 1; 32]), 0);
            let mut coin = key_wallet::Utxo::new(
                *point,
                TxOut {
                    value: 1_000_000,
                    script_pubkey: address.script_pubkey(),
                },
                address.clone(),
                1_570_000,
                false,
            );
            match n {
                0 => coin.is_confirmed = true,
                1 | 2 => coin.is_instantlocked = true,
                3 => coin.is_trusted = true,
                _ => {
                    coin.is_confirmed = true;
                    coin.lock();
                }
            }
            account.utxos.insert(*point, coin);
        }
        (info, points)
    }

    fn locked(info: &ManagedWalletInfo) -> Vec<OutPoint> {
        let mut out: Vec<_> = info.accounts().standard_bip44_accounts[&0]
            .utxos
            .values()
            .filter(|u| u.is_locked)
            .map(|u| u.outpoint)
            .collect();
        out.sort();
        out
    }

    /// AC-T3.1, AC-T3.2, AC-T3.5: the coin of the lock not verified is frozen (so is our change
    /// for an asset lock); the confirmed one and the verified one stay free. AC-T3.4: freed after,
    /// the payment's own lock untouched.
    #[test]
    fn coins_not_final_are_frozen_for_the_build_and_freed_after() {
        let (mut info, [confirmed, verified, unverified, change, reserved]) = wallet();
        let is_verified = |txid: &Txid| *txid == verified.txid;
        let mut held = HashMap::new();
        let frozen = freeze_unfinal(&mut info, is_verified, false, &mut held);
        let mut want = vec![unverified, change];
        want.sort();
        assert_eq!(frozen, want);
        let mut now = vec![unverified, change, reserved];
        now.sort();
        assert_eq!(locked(&info), now);
        assert!(!locked(&info).contains(&confirmed));
        thaw(&mut info, &frozen, &mut held);
        assert_eq!(
            locked(&info),
            vec![reserved],
            "only the payment's own lock is left"
        );
        assert!(held.is_empty());
    }

    /// A payment to a contact may take our own change (R-5): it stays free there.
    #[test]
    fn a_contact_payment_keeps_our_change() {
        let (mut info, [_, verified, unverified, _, _]) = wallet();
        let mut held = HashMap::new();
        let frozen = freeze_unfinal(&mut info, |t| *t == verified.txid, true, &mut held);
        assert_eq!(frozen, vec![unverified]);
    }

    /// Two builds at once: the coin stays frozen until the last one is done.
    #[test]
    fn a_coin_two_builds_hold_is_freed_by_the_last() {
        let (mut info, [_, verified, unverified, _, _]) = wallet();
        let mut held = HashMap::new();
        let first = freeze_unfinal(&mut info, |t| *t == verified.txid, true, &mut held);
        let second = freeze_unfinal(&mut info, |t| *t == verified.txid, true, &mut held);
        assert_eq!(first, second);
        thaw(&mut info, &first, &mut held);
        assert!(
            locked(&info).contains(&unverified),
            "the second build still holds it"
        );
        thaw(&mut info, &second, &mut held);
        assert!(!locked(&info).contains(&unverified));
    }
}

/// BUG-232: an address of ours past the pool's gap is taken in.
#[cfg(test)]
mod gap_tests {
    use super::*;
    use key_wallet::managed_account::address_pool::{AddressPool, AddressPoolType, KeySource};
    use key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
    use key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const OTHER: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";

    /// The BIP44 account 0's change pool.
    fn change_pool(info: &ManagedWalletInfo) -> &AddressPool {
        info.accounts.standard_bip44_accounts[&0]
            .keys()
            .managed_account_type()
            .address_pools()
            .into_iter()
            .find(|p| p.pool_type == AddressPoolType::Internal)
            .unwrap()
    }

    /// The change address at `index`, made apart from the wallet.
    fn change_at(info: &ManagedWalletInfo, keys: &key_wallet::Wallet, index: u32) -> Address {
        let mut probe = change_pool(info).clone();
        probe.gap_limit = index + 1;
        probe
            .maintain_gap_limit(&KeySource::Public(
                keys.accounts.standard_bip44_accounts[&0].account_xpub,
            ))
            .unwrap();
        probe.address_at_index(index).unwrap()
    }

    /// qa-f29's tx1: a change 3 past the pool's last address is ours once taken in, the pool
    /// keeps its gap after it, and History's [`live_own`] calls it change. One far past the
    /// look-ahead, and one of another wallet, are left.
    #[test]
    fn a_change_past_the_gap_is_taken_in() {
        let keys = super::super::keys_of(PHRASE).unwrap();
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        let last = change_pool(&info).highest_generated.unwrap();
        let near = change_at(&info, &keys, last + 3);
        let far = change_at(&info, &keys, last + GAP_LOOKAHEAD + 40);
        let other_info = ManagedWalletInfo::from_wallet(&super::super::keys_of(OTHER).unwrap(), 0);
        let other = change_pool(&other_info).address_at_index(0).unwrap();
        assert!(!live_own(&info).contains_key(&near.script_pubkey()));
        let adopted = adopt_addresses_past_the_gap(
            &mut info,
            &keys,
            &[near.clone(), far.clone(), other.clone()],
        );
        assert_eq!(adopted, vec![near.clone()]);
        let pool = change_pool(&info);
        assert_eq!(pool.highest_used, Some(last + 3));
        // The usual gap after it, not the look-ahead (review S1).
        assert_eq!(pool.highest_generated, Some(last + 3 + pool.gap_limit));
        assert_eq!(
            live_own(&info).get(&near.script_pubkey()),
            Some(&OutputRole::Change)
        );
        assert!(!live_own(&info).contains_key(&other.script_pubkey()));
        assert!(!live_own(&info).contains_key(&far.script_pubkey()));
        // Taken in once: nothing more the second time.
        assert!(adopt_addresses_past_the_gap(&mut info, &keys, &[near]).is_empty());
    }

    /// BUG-232 end to end, qa-f29's tx1: our coin spent to our own receiving address and to a
    /// change 3 past the pool. Before: the change counts as sent («Sent», the change and the fee);
    /// after taking it in, the same record in the same run reads as a transfer between own
    /// addresses, the fee only, as the restart showed it.
    #[test]
    fn the_history_row_turns_into_a_self_transfer_in_the_same_run() {
        use key_wallet::dashcore::hashes::Hash;
        use key_wallet::dashcore::{OutPoint, Transaction, TxIn, TxOut, Txid};
        use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};
        let keys = super::super::keys_of(PHRASE).unwrap();
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        let last = change_pool(&info).highest_generated.unwrap();
        let change = change_at(&info, &keys, last + 3);
        let receive = info.accounts.standard_bip44_accounts[&0]
            .keys()
            .managed_account_type()
            .address_pools()
            .into_iter()
            .find(|p| p.pool_type == AddressPoolType::External)
            .and_then(|p| p.address_at_index(0))
            .unwrap();
        let spent_coin = receive.clone();
        let (fee, to_us, back) = (226u64, 10_000u64, 14_525_584u64);
        let tx = Transaction {
            version: 2,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: OutPoint::new(Txid::from_byte_array([9; 32]), 0),
                ..TxIn::default()
            }],
            output: vec![
                TxOut {
                    value: to_us,
                    script_pubkey: receive.script_pubkey(),
                },
                TxOut {
                    value: back,
                    script_pubkey: change.script_pubkey(),
                },
            ],
            special_transaction_payload: None,
        };
        let row = |info: &ManagedWalletInfo| {
            let mut r = TransactionRecord::new(
                tx.clone(),
                key_wallet::account::AccountType::Standard {
                    index: 0,
                    standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
                },
                TransactionContext::Mempool,
                TransactionType::Standard,
                TransactionDirection::Outgoing,
                vec![InputDetail {
                    index: 0,
                    value: to_us + back + fee,
                    address: spent_coin.clone(),
                }],
                vec![
                    OutputDetail {
                        index: 0,
                        role: OutputRole::Sent,
                        address: Some(receive.clone()),
                        value: to_us,
                    },
                    OutputDetail {
                        index: 1,
                        role: OutputRole::Sent,
                        address: Some(change.clone()),
                        value: back,
                    },
                ],
                -((to_us + back + fee) as i64),
            );
            let live = live_own(info);
            own_outputs(&mut r, |a| live.get(&a.script_pubkey()).copied());
            (activity(&r, 0, false).kind, r.net_amount)
        };
        let (before, net) = row(&info);
        assert!(
            !matches!(before, ActivityKind::SelfTransfer { .. }),
            "{before:?}"
        );
        assert_eq!(net, -((back + fee) as i64), "the change counted as sent");
        let paid = paid_outside(&[TransactionRecord::new(
            tx.clone(),
            key_wallet::account::AccountType::Standard {
                index: 0,
                standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
            },
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Outgoing,
            vec![],
            vec![OutputDetail {
                index: 1,
                role: OutputRole::Sent,
                address: Some(change.clone()),
                value: back,
            }],
            0,
        )]);
        let addresses: Vec<_> = paid.into_iter().map(|(a, _)| a).collect();
        assert_eq!(
            adopt_addresses_past_the_gap(&mut info, &keys, &addresses),
            vec![change.clone()]
        );
        let (after, net) = row(&info);
        assert!(
            matches!(after, ActivityKind::SelfTransfer { .. }),
            "{after:?}"
        );
        assert_eq!(net, -(fee as i64), "the fee only");
    }

    /// A pool that moved between the read and the write (a block grew it) is not overwritten.
    #[test]
    fn a_pool_that_moved_meanwhile_is_left() {
        let keys = super::super::keys_of(PHRASE).unwrap();
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        let last = change_pool(&info).highest_generated.unwrap();
        let near = change_at(&info, &keys, last + 3);
        let finds = find_past_the_gap(&info, &keys, std::slice::from_ref(&near));
        assert_eq!(finds.len(), 1);
        let first = change_pool(&info).address_at_index(0).unwrap();
        info.accounts
            .standard_bip44_accounts
            .get_mut(&0)
            .unwrap()
            .keys_mut()
            .managed_account_type_mut()
            .address_pools_mut()
            .into_iter()
            .find(|p| p.pool_type == AddressPoolType::Internal)
            .unwrap()
            .mark_used(&first);
        assert!(take_in_past_the_gap(&mut info, finds).is_empty());
        assert!(!live_own(&info).contains_key(&near.script_pubkey()));
    }

    /// A run of payments, each past the end the one before it leaves: both taken in.
    #[test]
    fn a_run_of_them_is_found() {
        let keys = super::super::keys_of(PHRASE).unwrap();
        let mut info = ManagedWalletInfo::from_wallet(&keys, 0);
        let last = change_pool(&info).highest_generated.unwrap();
        let first = change_at(&info, &keys, last + 90);
        let second = change_at(&info, &keys, last + 90 + 80);
        let adopted = adopt_addresses_past_the_gap(&mut info, &keys, &[second, first]);
        assert_eq!(adopted.len(), 2);
        assert_eq!(change_pool(&info).highest_used, Some(last + 170));
    }
}

/// BUG-237: [`merge_slices`], one rule for the store's write and the screen.
#[cfg(test)]
pub(crate) mod merge_slices_tests {
    use super::*;
    use key_wallet::dashcore::address::Payload;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{BlockHash, OutPoint, PubkeyHash, Transaction, TxIn, TxOut, Txid};
    use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};
    use key_wallet::transaction_checking::BlockInfo;

    pub(crate) const PAID: u64 = 10_000;
    pub(crate) const CHANGE: u64 = 14_525_584;
    pub(crate) const FEE: u64 = 226;

    fn address(n: u8) -> Address {
        Address::new(
            key_wallet::Network::Testnet,
            Payload::PubkeyHash(PubkeyHash::from_byte_array([n; 20])),
        )
    }

    /// The shape of qa-f31's tx1: one coin of ours in, 0,0001 to a DashPay contact (output 0),
    /// the change to our internal address (output 1). (BIP44's slice, the DashPay account's.)
    pub(crate) fn qa_f31() -> (TransactionRecord, TransactionRecord) {
        let (mine, change, contact) = (address(7), address(8), address(9));
        let tx = Transaction {
            version: 2,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: OutPoint::new(Txid::from_byte_array([1; 32]), 0),
                ..TxIn::default()
            }],
            output: vec![
                TxOut {
                    value: PAID,
                    script_pubkey: contact.script_pubkey(),
                },
                TxOut {
                    value: CHANGE,
                    script_pubkey: change.script_pubkey(),
                },
            ],
            special_transaction_payload: None,
        };
        let out = |index, role, at: &Address, value| OutputDetail {
            index,
            role,
            address: Some(at.clone()),
            value,
        };
        let mut bip44 = TransactionRecord::new(
            tx.clone(),
            key_wallet::account::AccountType::Standard {
                index: 0,
                standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
            },
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Outgoing,
            vec![InputDetail {
                index: 0,
                value: PAID + CHANGE + FEE,
                address: mine,
            }],
            vec![
                out(0, OutputRole::Sent, &contact, PAID),
                out(1, OutputRole::Change, &change, CHANGE),
            ],
            -((PAID + FEE) as i64),
        );
        // The spending account knows its fee.
        bip44.fee = Some(FEE);
        let dashpay = TransactionRecord::new(
            tx,
            key_wallet::account::AccountType::DashpayReceivingFunds {
                index: 0,
                user_identity_id: [2; 32],
                friend_identity_id: [3; 32],
            },
            TransactionContext::Mempool,
            TransactionType::Standard,
            TransactionDirection::Outgoing,
            Vec::new(),
            vec![out(0, OutputRole::Sent, &contact, PAID)],
            -(PAID as i64),
        );
        (bip44, dashpay)
    }

    /// What a merged record says, comparable: its account, inputs, outputs by role, net, fee.
    pub(crate) type Shape = (String, Vec<u32>, Vec<(u32, OutputRole)>, i64, Option<u64>);

    /// The [`Shape`] of `r`.
    pub(crate) fn shape(r: &TransactionRecord) -> Shape {
        (
            format!("{:?}", r.account_type),
            r.input_details.iter().map(|d| d.index).collect(),
            r.output_details.iter().map(|d| (d.index, d.role)).collect(),
            r.net_amount,
            r.fee,
        )
    }

    /// The store kept the DashPay slice and lost the change: merged, the row has the input, the
    /// payment and the change, under the spending account — whichever came first.
    #[test]
    fn the_dashpay_slice_does_not_lose_the_change() {
        let (bip44, dashpay) = qa_f31();
        let merged = merge_slices(&dashpay, &bip44);
        assert_eq!(merged.input_details.len(), 1);
        assert_eq!(
            shape(&merged).2,
            [(0, OutputRole::Sent), (1, OutputRole::Change)]
        );
        assert!(matches!(
            merged.account_type,
            key_wallet::account::AccountType::Standard { .. }
        ));
        assert_eq!(merged.net_amount, -((PAID + FEE) as i64));
        assert_eq!(merged.fee, Some(FEE));
        // The order does not matter.
        assert_eq!(shape(&merge_slices(&bip44, &dashpay)), shape(&merged));
    }

    /// Merged once is merged: the row merged again with either slice is the same.
    #[test]
    fn a_merge_is_idempotent() {
        let (bip44, dashpay) = qa_f31();
        let merged = merge_slices(&dashpay, &bip44);
        assert_eq!(shape(&merge_slices(&merged, &bip44)), shape(&merged));
        assert_eq!(shape(&merge_slices(&merged, &dashpay)), shape(&merged));
        assert_eq!(shape(&merge_slices(&bip44, &bip44)), shape(&bip44));
    }

    /// An output one slice owns and the other sees as `Sent` is ours, in either order.
    #[test]
    fn ours_wins_over_sent() {
        let (bip44, _) = qa_f31();
        let mut sees_sent = bip44.clone();
        sees_sent.output_details[1].role = OutputRole::Sent;
        sees_sent.account_type = key_wallet::account::AccountType::Standard {
            index: 1,
            standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
        };
        for merged in [
            merge_slices(&sees_sent, &bip44),
            merge_slices(&bip44, &sees_sent),
        ] {
            assert_eq!(merged.output_details[1].role, OutputRole::Change);
        }
    }

    /// The context is the most final of the two, not the newest: ChainLock > block >
    /// InstantLock > mempool.
    #[test]
    fn the_context_is_the_most_final() {
        let (bip44, dashpay) = qa_f31();
        let block = BlockInfo::new(100, BlockHash::from_byte_array([4; 32]), 0);
        let at = |r: &TransactionRecord, c: TransactionContext| {
            let mut r = r.clone();
            r.context = c;
            r
        };
        let locked = at(&bip44, TransactionContext::InChainLockedBlock(block));
        let mined = at(&dashpay, TransactionContext::InBlock(block));
        let instant = at(
            &dashpay,
            TransactionContext::InstantSend(Default::default()),
        );
        for (a, b) in [(&locked, &mined), (&mined, &locked)] {
            assert!(merge_slices(a, b).context.is_chain_locked());
        }
        let mempool = at(&bip44, TransactionContext::Mempool);
        for (a, b) in [(&mempool, &instant), (&instant, &mempool)] {
            assert!(matches!(
                merge_slices(a, b).context,
                TransactionContext::InstantSend(_)
            ));
        }
        assert!(
            merge_slices(&mined, &instant)
                .context
                .block_info()
                .is_some()
        );
    }

    /// An asset lock is not merged: its base stays as it is (the most final context aside).
    #[test]
    fn an_asset_lock_is_left_as_it_is() {
        let (mut bip44, mut dashpay) = qa_f31();
        bip44.transaction_type = TransactionType::AssetLock;
        dashpay.transaction_type = TransactionType::AssetLock;
        let merged = merge_slices(&dashpay, &bip44);
        assert_eq!(shape(&merged), shape(&bip44));
    }

    /// `merged_records`' shape: the store's short row first, then this run's slices — the
    /// line is the whole transaction, not the stored slice that hid the live one.
    #[test]
    fn a_short_stored_row_does_not_hide_the_live_slice() {
        let (bip44, dashpay) = qa_f31();
        let one = one_per_transaction([&dashpay, &bip44, &dashpay]);
        assert_eq!(one.len(), 1);
        assert_eq!(shape(&one[0]), shape(&merge_slices(&dashpay, &bip44)));
        assert_eq!(one[0].output_details.len(), 2);
    }

    /// Neither or both with inputs: the stored one (the first) is the base.
    #[test]
    fn without_a_spending_slice_the_stored_one_is_the_base() {
        let (bip44, dashpay) = qa_f31();
        let mut other = dashpay.clone();
        other.account_type = key_wallet::account::AccountType::Standard {
            index: 3,
            standard_account_type: key_wallet::account::StandardAccountType::BIP44Account,
        };
        assert_eq!(
            merge_slices(&dashpay, &other).account_type,
            dashpay.account_type
        );
        assert_eq!(
            merge_slices(&other, &dashpay).account_type,
            other.account_type
        );
        let mut spends_too = bip44.clone();
        spends_too.account_type = other.account_type;
        assert_eq!(
            merge_slices(&bip44, &spends_too).account_type,
            bip44.account_type
        );
    }
}
