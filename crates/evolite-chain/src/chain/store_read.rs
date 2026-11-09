//! What is read straight from the store, read only: the history, and the asset locks consumed.

use super::*;

/// The wallet's history as kept in the store at `store`. platform-wallet writes every record
/// there but does not load them back into the wallet (it would need the raw transactions), and
/// sync goes on from the stored height: without this, history is empty after a restart.
///
/// Read only, on a connection of its own, in the store's own format (`core_transactions`, a
/// bincode `TransactionRecord` per row — platform-wallet-storage's reader for it is not
/// public). A row that does not decode is left out; a store this does not understand gives no
/// history, never a wrong one.
///
/// A row is one account's view when the last write was (BUG-86): its outputs to an address of
/// another of the wallet's accounts are `Sent` there. They are ours again by the store's own
/// address pools ([`own_scripts`], `convert::own_outputs`).
pub fn stored_history(
    store: &std::path::Path,
    wallet: &WalletId,
) -> Result<Vec<key_wallet::managed_account::transaction_record::TransactionRecord>, String> {
    let conn = open_read_only(store)?;
    let mut records = stored_records(&conn, wallet)?;
    let own = own_scripts(&conn, wallet);
    for r in &mut records {
        crate::convert::own_outputs(r, |address| {
            own.get(address.script_pubkey().as_bytes()).copied()
        });
    }
    Ok(records)
}

pub(super) fn open_read_only(store: &std::path::Path) -> Result<rusqlite::Connection, String> {
    use rusqlite::OpenFlags;
    rusqlite::Connection::open_with_flags(
        store,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())
}

/// The scripts of the wallet's own addresses in the store (`core_address_pool`), with what an
/// output to each is to the wallet: change (the internal pool) or received. Only the accounts
/// that hold coins: a DashPay contact's own addresses (`dashpay_external`, where we pay them),
/// identity and provider keys, Platform addresses are not where a Core payment comes to us.
/// Nothing when the store has no such table: the records stay as stored.
pub(super) fn own_scripts(
    conn: &rusqlite::Connection,
    wallet: &WalletId,
) -> std::collections::HashMap<Vec<u8>, key_wallet::managed_account::transaction_record::OutputRole>
{
    use key_wallet::managed_account::transaction_record::OutputRole;
    let Ok(mut stmt) = conn.prepare(
        "SELECT script, pool_type FROM core_address_pool WHERE wallet_id = ?1 AND account_type \
         IN ('standard_bip44', 'standard_bip32', 'coinjoin', 'dashpay_receiving')",
    ) else {
        return Default::default();
    };
    let Ok(rows) = stmt.query_map(rusqlite::params![wallet.as_slice()], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
    }) else {
        return Default::default();
    };
    rows.filter_map(Result::ok)
        .map(|(script, pool)| {
            // platform-wallet-storage `core_pool::pool_type_to_i64`: 1 is the internal pool.
            let role = if pool == 1 {
                OutputRole::Change
            } else {
                OutputRole::Received
            };
            (script, role)
        })
        .collect()
}

/// The store's records as written, one per transaction.
pub(super) fn stored_records(
    conn: &rusqlite::Connection,
    wallet: &WalletId,
) -> Result<Vec<key_wallet::managed_account::transaction_record::TransactionRecord>, String> {
    // The store's own limit on a blob.
    const LIMIT: usize = platform_wallet_storage::SIZE_LIMIT_BYTES;
    let mut stmt = conn
        .prepare(
            "SELECT record_blob FROM core_transactions \
             WHERE wallet_id = ?1 AND record_blob IS NOT NULL AND length(record_blob) <= ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![wallet.as_slice(), LIMIT as i64], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .map_err(|e| e.to_string())?;
    let config = bincode::config::standard().with_limit::<LIMIT>();
    Ok(rows
        .filter_map(Result::ok)
        .filter_map(|blob| {
            bincode::serde::decode_from_slice(&blob, config)
                .ok()
                .filter(|(_, used)| *used == blob.len())
                .map(|(record, _)| record)
        })
        .collect())
}

/// The txids of the asset locks this data folder's store keeps as consumed by Platform (BUG-57
/// r1). platform-wallet-storage keeps a consumed row for good and never reloads it, so this is
/// how a finished top-up is known after a restart. Read only, like [`stored_history`]. The
/// outpoint is the store's own bincode (`blob::encode_outpoint`): the length 32, the txid's 32
/// bytes, then the vout as a varint — the txid is bytes 2 to 33 (`substr(outpoint, 2, 32)`, as
/// the store's own test says). A row of any other shape is left out.
pub fn consumed_locks(
    store: &std::path::Path,
    wallet: &WalletId,
) -> Result<HashSet<String>, String> {
    use key_wallet::dashcore::hashes::Hash;
    use rusqlite::OpenFlags;
    let conn = rusqlite::Connection::open_with_flags(
        store,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT substr(outpoint, 2, 32) FROM asset_locks WHERE wallet_id = ?1 \
             AND status = 'consumed' AND length(outpoint) >= 34 \
             AND substr(outpoint, 1, 1) = x'20'",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![wallet.as_slice()], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .map_err(|e| e.to_string())?;
    Ok(rows
        .filter_map(Result::ok)
        .filter_map(|txid| key_wallet::dashcore::Txid::from_slice(&txid).ok())
        .map(|txid| txid.to_string())
        .collect())
}
