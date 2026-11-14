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
            || s.filters
                .is_some_and(|f| s.filters_at_tip.is_some_and(|at| f.height >= at)))
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
            let parts = [s.filter_headers.max(stored), stored, f.height];
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
    keys: &std::collections::HashMap<String, String>,
    contacts: &[Contact],
) {
    for a in history {
        let ActivityKind::Sent { to } = &mut a.kind else {
            continue;
        };
        let Some(friend) = keys.get(to.as_str()) else {
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

/// Payments to each contact as the history shows them (after [`name_contact_payments`]): a
/// wallet restored from its phrase has none on record in platform-wallet, the chain has them.
pub fn count_contact_payments(
    history: &[Activity],
    keys: &std::collections::HashMap<String, String>,
    contacts: &mut [Contact],
) {
    for c in contacts
        .iter_mut()
        .filter(|c| c.state == ContactState::Established)
    {
        let to_them = history.iter().filter(|a| {
            matches!(a.kind, ActivityKind::Sent { .. })
                && a.address.as_ref().and_then(|ad| keys.get(ad)) == Some(&c.identity_id)
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

/// This run's records (one per account a transaction touches) as one per transaction. The base
/// is the first record with inputs (the spending account's), else the first. An asset lock stays
/// that record alone (its other account's details are of the lock's payload). Any other
/// transaction gets every account's inputs and outputs, once by index — an output one account
/// owns and another sees as `Sent` stays owned — and its net, direction and fee from them
/// (key-wallet's rules). Picking one account's record left a transfer to an own address that
/// spent a contact's coin as the contact account's view: the coin out, nothing back (BUG-85).
pub fn one_per_transaction<'a>(
    records: impl IntoIterator<Item = &'a TransactionRecord>,
) -> Vec<TransactionRecord> {
    let mut groups: Vec<Vec<&TransactionRecord>> = Vec::new();
    for r in records {
        match groups.iter_mut().find(|g| g[0].txid == r.txid) {
            Some(g) => g.push(r),
            None => groups.push(vec![r]),
        }
    }
    groups
        .into_iter()
        .map(|group| {
            let base = group
                .iter()
                .find(|r| !r.input_details.is_empty())
                .unwrap_or(&group[0]);
            let mut merged = (*base).clone();
            if group.len() == 1 || merged.transaction_type == TransactionType::AssetLock {
                return merged;
            }
            for r in &group {
                for d in &r.input_details {
                    if !merged.input_details.iter().any(|m| m.index == d.index) {
                        merged.input_details.push(d.clone());
                    }
                }
                for d in &r.output_details {
                    let owned = matches!(d.role, OutputRole::Received | OutputRole::Change);
                    match merged
                        .output_details
                        .iter_mut()
                        .find(|m| m.index == d.index)
                    {
                        Some(m)
                            if owned
                                && !matches!(m.role, OutputRole::Received | OutputRole::Change) =>
                        {
                            *m = d.clone()
                        }
                        Some(_) => {}
                        None => merged.output_details.push(d.clone()),
                    }
                }
            }
            merged.input_details.sort_by_key(|d| d.index);
            merged.output_details.sort_by_key(|d| d.index);
            settle_record(&mut merged);
            merged
        })
        .collect()
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
        ..Detail::default()
    }
}

/// A spend the wallet saw before the coin it spends. A restore finds a coin on an address
/// beyond the gap limit only after a rescan, and then the spending transaction is already
/// recorded — from its change alone, as "Received" the change. key-wallet attributes
/// such late inputs only to records it still keeps in memory; a chain-locked one is dropped by
/// then, and the store keeps it uncorrected. Here every record's inputs are matched against
/// the wallet's own outputs in the other records; a record that spends one of them gets the
/// input, the outputs that are not ours as `Sent`, and its net, direction and fee again (the
/// same rules as key-wallet's).
pub fn attribute_late_inputs(records: &mut [TransactionRecord]) {
    use key_wallet::dashcore::OutPoint;
    use key_wallet::managed_account::transaction_record::{InputDetail, OutputDetail};
    use std::collections::HashMap;

    let ours: HashMap<OutPoint, (u64, Address)> = records
        .iter()
        .flat_map(|r| {
            r.output_details
                .iter()
                .filter(|d| matches!(d.role, OutputRole::Received | OutputRole::Change))
                .filter_map(move |d| {
                    let address = d.address.clone()?;
                    Some((OutPoint::new(r.txid, d.index), (d.value, address)))
                })
        })
        .collect();
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
        // What the record did not list is not ours: paid to someone, or a data output.
        for (index, output) in r.transaction.output.iter().enumerate() {
            let index = index as u32;
            if r.output_details.iter().any(|d| d.index == index) {
                continue;
            }
            let role = if output.script_pubkey.is_provably_unspendable() {
                OutputRole::Unspendable
            } else {
                OutputRole::Sent
            };
            r.output_details.push(OutputDetail {
                index,
                role,
                address: Address::from_script(&output.script_pubkey, crate::network::dash()).ok(),
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

/// What the network wants on the identity before it writes a contact request (BUG-107): Platform
/// compares the balance with an estimate (`validate_fees_of_event`, a dry run at worst-case tree
/// costs), not with the fee, and then takes less. Manual pass 16 was refused at 126 042 duffs and
/// accepted at 226 042; where between is not known, this is above the accepted one.
const CONTACT_REQUEST_BALANCE: Duffs = 230_000;

/// A contact request on testnet, 2026-11-05: 0.00102028 DASH (manual run M-7), rounded up.
const CONTACT_REQUEST: Duffs = 105_000;
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
    }
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
        // Metered like the documents above: what remains of an identity's lock is shown
        // with a margin ([`Fees::name_on_top`]).
        name_regular: NAME_REGULAR,
        name_contested: 20_000_000,
        add_key: 10_000,
        contact_request: CONTACT_REQUEST,
        contact_request_balance: CONTACT_REQUEST_BALANCE,
        // Not measured yet: one document like a contact request, with fewer indexes — this is
        // the upper side.
        profile: CONTACT_REQUEST,
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
    fees.credit_withdrawal = journal
        .last_fee(|w| matches!(w, What::Withdrawal { .. }))
        .unwrap_or(duffs(min.credit_withdrawal));
    fees.credit_withdrawal_reserve = duffs(min.credit_withdrawal);
    if let Some(fee) = journal.last_fee(|w| matches!(w, What::ContactRequest { .. })) {
        fees.contact_request = fee;
    }
    if let Some(fee) = journal.last_fee(|w| matches!(w, What::Profile)) {
        fees.profile = fee;
    }
    if let Some(fee) =
        journal.last_fee(|w| matches!(w, What::ContactRequest { .. } | What::Profile))
    {
        fees.document_write = fee;
    }
    if let Some(fee) = journal.last_fee(|w| matches!(w, What::Name { .. })) {
        fees.name_regular = fee;
    }
    fees.name_contested = fees.name_regular
        + duffs(
            version
                .fee_version
                .vote_resolution_fund_fees
                .contested_document_vote_resolution_fund_required_amount,
        );
    // An identity is made with six keys (`identity::new_identity_keys`).
    fees.identity_lock_min = identity_lock_min(version, 6);
    fees.top_up_lock_min = top_up_lock_min(version);
    fees.add_key = duffs(min.identity_update + min.identity_key_in_creation_cost);
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
        assert!(near(f.profile), "profile ≈ {}", f.profile);
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
            f.name_regular + 10_000_000,
            "plus the vote fund"
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
        platform_fees(&mut f, v15, &journal);
        assert_eq!(
            (f.name_regular, f.credit_transfer),
            (123_456, CREDIT_TRANSFER)
        );
        let _ = std::fs::remove_dir_all(&dir);
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
        attribute_late_inputs(&mut records);
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
        };
        let keys = [(contact_key.to_string(), "Jonas111".to_owned())].into();
        let mut history = vec![line];
        name_contact_payments(&mut history, &keys, std::slice::from_ref(&contact));
        assert!(matches!(&history[0].kind, ActivityKind::Sent { to } if to == "@bob"));
        let mut contacts = vec![contact];
        count_contact_payments(&history, &keys, &mut contacts);
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
        };
        let contact = |id: &str, name: Option<&str>| Contact {
            identity_id: id.into(),
            name: name.map(Into::into),
            profile: Default::default(),
            state: ContactState::Established,
            since: 0,
            payments: 0,
            last_payment: None,
        };
        let keys = [
            ("yLcB1".to_owned(), "Abc111".to_owned()),
            ("yev72".to_owned(), "Xyz9999888".to_owned()),
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
        count_contact_payments(&h, &keys, &mut contacts);
        assert_eq!((contacts[0].payments, contacts[1].payments), (1, 1));
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
        attribute_late_inputs(&mut records);
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
            attribute_late_inputs(&mut records);
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
