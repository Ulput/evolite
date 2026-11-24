//! BUG-279 (the security audit's F3), a restart in the middle: an unconfirmed spend of our
//! coin froze it in the run only until it left the mempool (key-wallet gives the coin back on
//! expiry). platform-wallet restores the coins after a restart but not the records (its
//! `tx-record-rehydration-gap`), so the coin came back spent with nothing to expire. Here the
//! store's own records, read as History reads them after a restart ([`store_read`], read only,
//! nothing written), hold such a spend again as it was: the coin out of the balance, the spend
//! among the wallet's unconfirmed transactions, which the mempool tracks for expiry.
//!
//! BUG-279 r1 (Must): the store keeps no time of the first sight, so the time is kept here, in
//! the wallet's own `held-since.json` (txid → seconds since 1970). Counted from each load,
//! a peer could keep a coin frozen for ever against a wallet opened once a day. The first
//! sight goes to the wallet for the mempool's 24 h ([`Chain::keep_held_since`]).

use super::*;
use key_wallet::dashcore::Txid;
use key_wallet::managed_account::transaction_record::{OutputRole, TransactionRecord};
use key_wallet::transaction_checking::TransactionContext;
use key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
use std::collections::{BTreeMap, HashMap, HashSet};

impl Chain {
    /// The store's unconfirmed spends of `wallet`'s coins, held again in the wallet just
    /// loaded. How many. A store that can't be read holds none (as History then shows none).
    pub(super) async fn hold_stored_spends(&self, wallet: &PlatformWallet) -> usize {
        let store = self
            .persister
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|store| store.db_path().to_path_buf());
        let Some(store) = store else {
            return 0;
        };
        let id = wallet.wallet_id();
        let Ok(records) = store_read::stored_history(&store, &id) else {
            return 0;
        };
        let mut manager = wallet.wallet_manager().write().await;
        let Some(info) = manager.get_wallet_info_mut(&id) else {
            return 0;
        };
        let held = hold_again(info, &records);
        if held > 0 {
            info.update_balance();
        }
        drop(manager);
        self.keep_held_since(wallet).await;
        held
    }

    /// BUG-279 r1: `held-since.json` in step with `wallet`'s unconfirmed transactions: each
    /// keeps the time it was first seen (an earlier one is never replaced; one new gets now),
    /// those times go to the wallet for the mempool's expiry, and an entry for one no longer
    /// unconfirmed (settled, or expired) goes. A file that doesn't read is set aside and
    /// journaled by [`evolite_local::json_file::read_cache`], and the times count from now, as
    /// before the file.
    pub async fn keep_held_since(&self, wallet: &PlatformWallet) {
        let path = self
            .held_since
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(path) = path else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default(); // default-ok: a clock before 1970: every time is «the first»
        let id = wallet.wallet_id();
        let mut manager = wallet.wallet_manager().write().await;
        let Some(info) = manager.get_wallet_info(&id) else {
            return;
        };
        let unconfirmed: Vec<Txid> = info
            .unconfirmed_transactions()
            .iter()
            .map(|tx| tx.txid())
            .collect();
        let before = read_held_since(&path);
        let kept = held_since_now(&before, &unconfirmed, now);
        for (txid, since) in &kept {
            manager.note_first_seen(*txid, *since);
        }
        if kept != before {
            let text: BTreeMap<String, u64> = kept
                .iter()
                .map(|(txid, since)| (txid.to_string(), *since))
                .collect();
            let _ = evolite_local::json_file::save(&path, &text);
        }
    }
}

/// `held-since.json`, by txid. A row that is not a txid is left out.
fn read_held_since(path: &std::path::Path) -> BTreeMap<Txid, u64> {
    evolite_local::json_file::read_cache(path, |bytes| {
        serde_json::from_slice::<BTreeMap<String, u64>>(bytes).ok()
    })
    .unwrap_or_default() // default-ok: no file, or one set aside: from now, as before the file
    .into_iter()
    .filter_map(|(txid, since)| Some((txid.parse().ok()?, since)))
    .collect()
}

/// What `held-since.json` keeps for `unconfirmed` at `now`: each one's first sight as
/// `before` has it, or `now` for one new; nothing for a transaction no longer unconfirmed.
fn held_since_now(
    before: &BTreeMap<Txid, u64>,
    unconfirmed: &[Txid],
    now: u64,
) -> BTreeMap<Txid, u64> {
    unconfirmed
        .iter()
        .map(|txid| (*txid, before.get(txid).copied().unwrap_or(now).min(now)))
        .collect()
}

/// Of `records` (the store's), each unconfirmed one the wallet does not hold that spends a coin
/// of ours (an output to our own address of another stored record) is held again with those
/// coins ([`WalletInfoInterface::resume_unsettled_spend`]). How many.
pub(super) fn hold_again(
    info: &mut impl WalletInfoInterface,
    records: &[TransactionRecord],
) -> usize {
    let held: HashSet<Txid> = info
        .accounts()
        .all_accounts()
        .into_iter()
        .flat_map(|account| account.transactions().keys().copied().collect::<Vec<_>>())
        .collect();
    let by_txid: HashMap<Txid, &TransactionRecord> = records.iter().map(|r| (r.txid, r)).collect();
    let mut count = 0;
    for record in records {
        let settled = record.is_confirmed()
            || matches!(record.context, TransactionContext::InstantSend(_))
            || record.context.is_chain_locked();
        if settled || held.contains(&record.txid) {
            continue;
        }
        let coins: Vec<key_wallet::Utxo> = record
            .transaction
            .input
            .iter()
            .filter_map(|input| {
                let outpoint = input.previous_output;
                let funding = by_txid.get(&outpoint.txid)?;
                let detail = funding.output_details.iter().find(|d| {
                    d.index == outpoint.vout
                        && matches!(d.role, OutputRole::Received | OutputRole::Change)
                })?;
                let txout = funding
                    .transaction
                    .output
                    .get(outpoint.vout as usize)?
                    .clone();
                let mut utxo = key_wallet::Utxo::new(
                    outpoint,
                    txout,
                    detail.address.clone()?,
                    funding.height().unwrap_or_default(), // default-ok: unconfirmed funding has no height
                    false,
                );
                utxo.is_confirmed = funding.is_confirmed();
                utxo.is_instantlocked =
                    matches!(funding.context, TransactionContext::InstantSend(_));
                Some(utxo)
            })
            .collect();
        // Spends nothing of ours: nothing frozen, nothing to give back.
        if coins.is_empty() {
            continue;
        }
        if info.resume_unsettled_spend(record.clone(), coins) {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn paths(name: &str) -> ChainPaths {
        let dir =
            std::env::temp_dir().join(format!("evolite-bug279-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ChainPaths {
            spv: dir.join("spv").join("testnet"),
            store: dir
                .join("wallets")
                .join("testnet")
                .join("platform-wallet.db"),
            core_node: dir.join("core-node.txt"),
            held_since: dir.join("held-since.json"),
        }
    }
    use key_wallet::account::{AccountType, StandardAccountType};
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{OutPoint, ScriptBuf, Transaction, TxIn, TxOut, Witness};
    use key_wallet::managed_account::transaction_record::{
        InputDetail, OutputDetail, TransactionDirection,
    };
    use key_wallet::transaction_checking::{BlockInfo, TransactionType};
    use platform_wallet::changeset::{
        CoreChangeSet, PlatformWalletChangeSet, PlatformWalletPersistence,
    };

    const AMOUNT: u64 = 100_000;

    /// BUG-279 r1: the first sight kept stays (20 h, 25 h before), one new is now, one no
    /// longer unconfirmed goes, and a later time never replaces an earlier one.
    #[test]
    fn the_first_sight_is_kept_and_a_settled_one_goes() {
        let (a, b, c) = (
            Txid::from_byte_array([1; 32]),
            Txid::from_byte_array([2; 32]),
            Txid::from_byte_array([3; 32]),
        );
        let now = 1_800_000_000;
        let before = BTreeMap::from([(a, now - 20 * 3600), (b, now - 25 * 3600), (c, now - 60)]);
        let kept = held_since_now(&before, &[a, b, Txid::from_byte_array([4; 32])], now);
        assert_eq!(
            kept[&a],
            now - 20 * 3600,
            "20 h before: kept, so 4 h are left"
        );
        assert_eq!(
            kept[&b],
            now - 25 * 3600,
            "25 h before: kept, so it expires at once"
        );
        assert_eq!(kept[&Txid::from_byte_array([4; 32])], now, "new: now");
        assert!(!kept.contains_key(&c), "settled or expired: gone");
    }

    /// BUG-279, a restart in the middle, on a store: a peer's unconfirmed spend of our coin,
    /// stored with the coin spent, is held again when the wallet loads: the coin out of the
    /// balance, the spend among the unconfirmed for the mempool to track, and its expiry gives
    /// the coin back. Mined before the restart, nothing is held and nothing comes back.
    #[tokio::test(flavor = "multi_thread")]
    async fn after_a_restart_a_pending_spend_is_held_again_and_expires() {
        for mined in [false, true] {
            let paths = paths(if mined { "held-mined" } else { "held" });
            let seed = crate::seed_from(PHRASE, "").unwrap();
            let chain = Chain::default();
            chain.open_store_only(&paths).await.unwrap();
            let (id, wallet) = chain.open_wallet(&seed, 0).await.unwrap();
            let address = {
                let manager = wallet.wallet_manager().read().await;
                manager.get_wallet_info(&id).unwrap().monitored_addresses()[0].clone()
            };
            let account = AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            };
            let funding = Transaction {
                version: 2,
                lock_time: 0,
                input: vec![TxIn {
                    previous_output: OutPoint::new(Txid::from_byte_array([0xaa; 32]), 0),
                    script_sig: ScriptBuf::new(),
                    sequence: u32::MAX,
                    witness: Witness::default(),
                }],
                output: vec![TxOut {
                    value: AMOUNT,
                    script_pubkey: address.script_pubkey(),
                }],
                special_transaction_payload: None,
            };
            let coin = OutPoint::new(funding.txid(), 0);
            // A peer's spend: unsigned, to an address not ours.
            let spend = Transaction {
                version: 2,
                lock_time: 0,
                input: vec![TxIn {
                    previous_output: coin,
                    script_sig: ScriptBuf::new(),
                    sequence: u32::MAX,
                    witness: Witness::default(),
                }],
                output: vec![TxOut {
                    value: AMOUNT,
                    script_pubkey: ScriptBuf::new_p2pkh(
                        &key_wallet::dashcore::PubkeyHash::from_byte_array([0x77; 20]),
                    ),
                }],
                special_transaction_payload: None,
            };
            let block = |height: u32| {
                TransactionContext::InBlock(BlockInfo::new(
                    height,
                    key_wallet::dashcore::BlockHash::from_byte_array([height as u8; 32]),
                    0,
                ))
            };
            let funding_record = TransactionRecord::new(
                funding.clone(),
                account,
                block(100),
                TransactionType::Standard,
                TransactionDirection::Incoming,
                Vec::new(),
                vec![OutputDetail {
                    index: 0,
                    role: OutputRole::Received,
                    address: Some(address.clone()),
                    value: AMOUNT,
                }],
                AMOUNT as i64,
            );
            let spend_record = TransactionRecord::new(
                spend.clone(),
                account,
                if mined {
                    block(101)
                } else {
                    TransactionContext::Mempool
                },
                TransactionType::Standard,
                TransactionDirection::Outgoing,
                vec![InputDetail {
                    index: 0,
                    value: AMOUNT,
                    address: address.clone(),
                }],
                vec![OutputDetail {
                    index: 0,
                    role: OutputRole::Sent,
                    address: None,
                    value: AMOUNT,
                }],
                -(AMOUNT as i64),
            );
            let mut utxo =
                key_wallet::Utxo::new(coin, funding.output[0].clone(), address.clone(), 100, false);
            utxo.is_confirmed = true;
            let store = chain.persister.lock().unwrap().clone().unwrap();
            store
                .store(
                    id,
                    PlatformWalletChangeSet {
                        core: Some(CoreChangeSet {
                            records: vec![funding_record, spend_record],
                            new_utxos: vec![utxo.clone()],
                            spent_utxos: vec![utxo],
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )
                .unwrap();
            drop(store);
            drop(wallet);
            chain.stop().await;

            // First seen 20 h ago, as kept by an earlier run.
            let first_sight = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                - 20 * 3600;
            if !mined {
                let kept = BTreeMap::from([(spend.txid().to_string(), first_sight)]);
                evolite_local::json_file::save(&paths.held_since, &kept).unwrap();
            }

            // The restart.
            let again = Chain::default();
            again.open_store_only(&paths).await.unwrap();
            let (_, loaded) = again.open_wallet(&seed, 0).await.unwrap();
            let mut manager = loaded.wallet_manager().write().await;
            let info = manager.get_wallet_info_mut(&id).unwrap();
            assert!(
                !info.utxos().iter().any(|u| u.outpoint == coin),
                "spent as stored"
            );
            let unconfirmed: Vec<Txid> = info
                .unconfirmed_transactions()
                .iter()
                .map(|tx| tx.txid())
                .collect();
            if mined {
                assert!(unconfirmed.is_empty(), "mined: nothing held");
            } else {
                assert_eq!(
                    unconfirmed,
                    vec![spend.txid()],
                    "held for the mempool to track"
                );
            }

            if !mined {
                // BUG-279 r1: the first sight kept 20 h ago stays, and the wallet hands the
                // mempool that time, not this load's: 4 h are left, not 24.
                let kept = read_held_since(&paths.held_since);
                assert_eq!(
                    kept.get(&spend.txid()),
                    Some(&first_sight),
                    "not this load's"
                );
                assert_eq!(manager.first_seen(&spend.txid()), Some(first_sight));
            } else {
                assert!(
                    read_held_since(&paths.held_since).is_empty(),
                    "settled: no entry"
                );
            }
            let info = manager.get_wallet_info_mut(&id).unwrap();
            let expiry = info.expire_mempool_transaction(spend.txid());
            info.update_balance();
            assert_eq!(expiry.restored.contains(&coin), !mined);
            assert_eq!(info.utxos().iter().any(|u| u.outpoint == coin), !mined);
            assert_eq!(info.balance().total(), if mined { 0 } else { AMOUNT });
        }
    }
}
