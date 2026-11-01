//! What dash-spv and key-wallet report, in the API's terms: sync stages, history, fees, and
//! the one address check before paying.

use evolite_api::*;
use key_wallet::dashcore::address::{Address, NetworkUnchecked};
use key_wallet::managed_account::transaction_record::{
    OutputRole, TransactionDirection, TransactionRecord,
};
use key_wallet::transaction_checking::TransactionContext;
use key_wallet::transaction_checking::transaction_router::TransactionType;
use key_wallet::wallet::managed_wallet_info::fee::{FeeRate, estimate_tx_size};

use super::sync::SyncStatus;

/// The wallet's coins are known: headers at the tip and, for an open wallet, its filters too.
/// Until the first filter report after a restart there is none, and that is "not yet".
pub(super) fn coins_known(s: &SyncStatus, wallet: bool) -> bool {
    s.headers.is_some_and(|h| h.done) && (!wallet || s.filters.is_some_and(|f| f.done))
}

/// What dash-spv reports, in the terms of the screens. `wallet`: a wallet is open, so its
/// filters are part of the sync.
pub(super) fn sync_info(s: &SyncStatus, wallet: bool) -> SyncInfo {
    let headers = s.headers;
    let tip = headers.map_or(0, |h| h.target);
    let ratio = |height: u32, target: u32| height as f32 / target.max(1) as f32;
    let (stage, progress) = match (headers, s.masternodes, s.filters) {
        // Nothing heard from the network yet (or it failed before that).
        (None, _, _) => (SyncStage::Connecting, 0.0),
        (Some(h), _, _) if !h.done => (SyncStage::Headers, ratio(h.height, h.target)),
        (_, Some(m), _) if !m.done => (SyncStage::Masternodes, ratio(m.height, m.target)),
        // Half for filters downloaded, half for batches done: the done height alone stood
        // at 28 % for minutes while a restored wallet's blocks came, then jumped to the end.
        (_, _, Some(f)) if wallet && !f.done => (
            SyncStage::Filters,
            (ratio(s.filters_stored.max(f.height), f.target) + ratio(f.height, f.target)) / 2.0,
        ),
        (_, _, None) if wallet => (SyncStage::Filters, 0.0),
        _ if s.chainlock_height.is_none() => (SyncStage::ChainLock, 1.0),
        _ => (SyncStage::Done, 1.0),
    };
    SyncInfo {
        stage,
        progress: progress.clamp(0.0, 1.0),
        tip_height: tip,
        headers_height: headers.map_or(0, |h| h.height),
        filters_height: s.filters.map_or(0, |f| f.height),
        core_peers: s.peers as u32,
        platform_nodes: 0,
        minutes_left: None,
        // A checked ChainLock near the tip: the history we show cannot be rewritten.
        chainlocked: s.chainlock_height.is_some_and(|c| tip > 0 && c + 10 >= tip),
        error: s.error.clone(),
        // The core counts the folder itself.
        disk_bytes: None,
        core_node: s.core_node.clone(),
        core_node_next: None,
        // The core asks the client and the SDK itself.
        core_nodes: Vec::new(),
        dapi_nodes: Vec::new(),
    }
}

/// A testnet Core address to pay to; a mainnet one is refused, not converted.
pub(super) fn payee(to: &str) -> Result<Address<NetworkUnchecked>, CoreError> {
    to.trim()
        .parse::<Address<NetworkUnchecked>>()
        .ok()
        .filter(|a| a.is_valid_for_network(crate::network::dash()))
        .ok_or(CoreError::BadAddress)
}

/// Why `to` is no address at all — a typo, a missing or extra character (its checksum does
/// not add up) — said apart from a good address of another network, which stays `BadAddress`.
pub(super) fn not_an_address(to: &str) -> Option<CoreError> {
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
pub(super) fn name_contact_payments(
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
pub(super) fn count_contact_payments(
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
pub(super) fn settle(history: &mut [Activity], chainlocked: Option<u32>) {
    let Some(locked) = chainlocked else {
        return;
    };
    for a in history {
        if a.proof == Proof::InBlock && a.block.is_some_and(|b| b <= locked) {
            a.proof = Proof::ChainLock;
        }
    }
}

fn proof(context: &TransactionContext) -> Proof {
    match context {
        TransactionContext::Mempool => Proof::Mempool,
        TransactionContext::InstantSend(_) => Proof::InstantLock,
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
            (ActivityKind::SelfTransfer, Chain::Core)
        }
    }
}

pub(super) fn activity(record: &TransactionRecord, seen: Timestamp) -> Activity {
    let sent_to = record
        .output_details
        .iter()
        .find(|o| o.role == OutputRole::Sent)
        .and_then(|o| o.address.as_ref())
        .map(|a| a.to_string());
    // Paid out: the payee's address; received: our own address it came to ("Receive" lists them).
    let address = if record.direction == TransactionDirection::Incoming {
        record
            .output_details
            .iter()
            .find(|o| o.role == OutputRole::Received)
            .and_then(|o| o.address.as_ref())
            .map(|a| a.to_string())
    } else {
        sent_to.clone()
    };
    let (kind, chain) = kind_of(record.direction, record.transaction_type, sent_to);
    // In a block: the block's time; in the mempool: when we saw it.
    let at = record
        .context
        .block_info()
        .map_or(seen, |b| b.timestamp() as Timestamp);
    let txid = record.txid.to_string();
    Activity {
        id: u64::from_str_radix(&txid[..16.min(txid.len())], 16).unwrap_or(0),
        kind,
        chain,
        at,
        amount: record.net_amount,
        proof: proof(&record.context),
        txid: Some(txid),
        // Only what this wallet paid: an incoming payment's fee is the sender's. An asset lock is
        // always this wallet's, whatever its net says (see `Real::wallet` on locks for the pool).
        fee: (record.net_amount < 0 || record.transaction_type == TransactionType::AssetLock)
            .then_some(record.fee)
            .flatten()
            .map(|f| f as Duffs),
        block: record.context.block_info().map(|b| b.height()),
        address,
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
pub(super) fn attribute_late_inputs(records: &mut [TransactionRecord]) {
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
pub(super) fn fee_rate() -> FeeRate {
    FeeRate::normal()
}

/// `core_payment`: the fee of a payment spending every coin (with change) — the most a
/// payment from this wallet can cost, so "Max" always leaves room for it. Platform fees are the
/// published costs.
pub(super) fn fees(coins: usize) -> Fees {
    Fees {
        core_payment: fee_rate().calculate_fee(estimate_tx_size(coins.max(1), 1, true)) as Duffs,
        credit_transfer: 2_000,
        nft_transfer: 30_000,
        document_write: 20_000,
        identity_create: 2_000,
        name_regular: 1_000_000,
        name_contested: 20_000_000,
        add_key: 10_000,
        // Measured on testnet would be better; a contact request and a profile are one document each.
        contact_request: 10_000,
        profile: 15_000,
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
    }
}

/// The fees of Platform writes outside the pool, at protocol `version`. Credit transfers,
/// withdrawals and documents (profile, contact request, name) are metered by Platform: what the
/// last one of a kind really cost (the journal), else the protocol's minimum — for documents,
/// whose minimum is far below any real write, the estimate above. Creating an identity and
/// adding a key have fixed protocol costs; a contested name adds the masternode vote's fund.
pub(super) fn platform_fees(
    fees: &mut Fees,
    version: &dpp::version::PlatformVersion,
    journal: &super::journal::Journal,
) {
    use super::journal::What;
    let duffs = |credits: u64| credits.div_ceil(1000) as Duffs;
    let min = &version.fee_version.state_transition_min_fees;
    fees.credit_transfer = journal
        .last_fee(|w| matches!(w, What::Credits { .. }))
        .unwrap_or(duffs(min.credit_transfer));
    fees.credit_withdrawal = journal
        .last_fee(|w| matches!(w, What::Withdrawal { .. }))
        .unwrap_or(duffs(min.credit_withdrawal));
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
    fees.identity_create =
        duffs(min.identity_create_base_cost + 6 * min.identity_key_in_creation_cost);
    fees.add_key = duffs(min.identity_update + min.identity_key_in_creation_cost);
}

#[cfg(test)]
mod tests {
    use super::super::sync::{Phase, Stage};
    use super::*;

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
        assert_eq!(f.credit_transfer, 100);
        assert_eq!(f.identity_create, 41_000, "base and six keys");
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
        });
        journal.add(super::super::journal::Entry {
            at: 2,
            what: super::super::journal::What::Credits { to: "@b".into() },
            spent: 10_777,
            moved: 10_000,
        });
        platform_fees(&mut f, v15, &journal);
        assert_eq!((f.name_regular, f.credit_transfer), (123_456, 777));
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
        let line = activity(&records[0], 0);
        assert_eq!(line.amount, -100_226, "what left the wallet");
        assert_eq!(line.fee, Some(226));
        assert!(
            matches!(&line.kind, ActivityKind::Sent { to } if *to == contact_key.to_string()),
            "{:?}",
            line.kind
        );
        // The coin's own line is as it was.
        assert_eq!(activity(&records[1], 0).amount, 998_000);

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

    #[test]
    fn the_fee_grows_with_the_coins_spent() {
        let one = fees(1).core_payment;
        assert!((200..400).contains(&one), "one coin: {one} duffs");
        assert!(fees(10).core_payment > one);
        assert_eq!(fees(0).core_payment, one);
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
        assert!((info.progress - 0.5).abs() < 1e-6);
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
        assert_eq!(sync_info(&s, true).stage, SyncStage::Done);
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
        let waiting = sync_info(&s, true).progress;
        s.filters_stored = 1_400_000;
        let info = sync_info(&s, true);
        assert_eq!(info.stage, SyncStage::Filters);
        // 28 % before, 58 % now: half of the stage is the download, and it moved.
        assert!(
            info.progress > waiting + 0.25,
            "{} vs {waiting}",
            info.progress
        );
        assert!(info.progress < 1.0, "not done until the batches are");
        // What is done stays what the history shows.
        assert_eq!(info.filters_height, 447_000);
    }
}

#[cfg(test)]
mod kind_tests {
    use super::*;

    #[test]
    fn tells_asset_locks_unlocks_and_payees_apart() {
        let out = TransactionDirection::Outgoing;
        assert_eq!(
            kind_of(out, TransactionType::AssetLock, None),
            (
                ActivityKind::TopUp {
                    identity: String::new()
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
            ActivityKind::SelfTransfer
        );
    }
}
