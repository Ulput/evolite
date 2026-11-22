//! BUG-275 (the architect's decision, 2026-11-22): where a Core payout from Platform came from,
//! proved from the network on any copy — not from the copy that made the withdrawal.
//!
//! - **The link:** an asset unlock's payload index is its withdrawal document's
//!   `transactionIndex` (drive-abci sets it when it builds the transaction). The document is
//!   read with proof by the Withdrawals contract's `transaction` index, and kept only when its
//!   `outputScript` is the unlock's: it gives the net amount and `$createdAt`.
//! - **The origin, proved only:** owned by one of the wallet's identities — a withdrawal of its
//!   credits; owned by the contract (`OWNER_ID`) with the id a shielded withdrawal of one of our
//!   spent notes makes (`nullifier ‖ script`, Drive's `shielded_withdrawal` transformer) — out of
//!   the pool; anything else — «from Platform», never guessed by amount or time.

use std::sync::Arc;

use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::DocumentQuery;
use dpp::data_contract::DataContract;
use dpp::data_contracts::withdrawals_contract;
use dpp::data_contracts::withdrawals_contract::v1::document_types::withdrawal;
use dpp::document::{Document, DocumentV0Getters};
use dpp::platform_value::Value;
use dpp::prelude::Identifier;

/// Statuses of a withdrawal whose transaction went to Core, the likeliest first: complete,
/// broadcast, expired (a rebroadcast keeps the document; the chain holds the unlock of its
/// current index).
pub const SENT: [u8; 3] = [3, 2, 4];

/// A withdrawal document as the link needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WithdrawalDoc {
    pub id: Identifier,
    pub owner: Identifier,
    /// Net of Platform's fee, in credits.
    pub amount: u64,
    /// Milliseconds since the epoch.
    pub created_ms: Option<u64>,
    pub script: Vec<u8>,
}

/// Where a payout came from, as proved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Out of the pool: the spent note whose nullifier makes the document's id.
    Pool { nullifier: [u8; 32] },
    /// One of the wallet's identities withdrew its credits.
    Identity(Identifier),
    /// From Platform, its source not proved (a Platform address, or a pool spend whose first
    /// action was a dummy).
    Platform,
}

/// The id of the document a shielded withdrawal makes (Drive: `shielded_withdrawal` v0
/// transformer): `generate_document_id_v0(Withdrawals, OWNER_ID, "withdrawal", nf ‖ script)`.
pub fn pool_withdrawal_id(nullifier: &[u8; 32], script: &[u8]) -> Identifier {
    let mut entropy = Vec::with_capacity(32 + script.len());
    entropy.extend_from_slice(nullifier);
    entropy.extend_from_slice(script);
    Document::generate_document_id_v0(
        &withdrawals_contract::ID,
        &withdrawals_contract::OWNER_ID,
        withdrawal::NAME,
        &entropy,
    )
}

/// The withdrawal documents of `index` in `status`: the contract's `transaction` (`status`,
/// `transactionIndex`) index, in the very form Drive asks it (r1 Must 2, Drive's
/// `find_withdrawal_documents_by_status_and_transaction_indices`): `status ==`,
/// `transactionIndex in`, ordered by `transactionIndex`.
pub fn by_index(
    withdrawals: Arc<DataContract>,
    status: u8,
    index: u64,
) -> Result<DocumentQuery, String> {
    Ok(DocumentQuery::new(withdrawals, withdrawal::NAME)
        .map_err(|e| format!("{e:?}"))?
        .with_where(WhereClause {
            field: "status".into(),
            operator: WhereOperator::Equal,
            value: Value::U8(status),
        })
        .with_where(WhereClause {
            field: "transactionIndex".into(),
            operator: WhereOperator::In,
            value: Value::Array(vec![Value::U64(index)]),
        })
        .with_order_by(dash_sdk::drive::query::OrderClause {
            field: "transactionIndex".into(),
            ascending: true,
        })
        .with_limit(10))
}

/// A withdrawal document's fields; `None` without them.
pub fn withdrawal_of(doc: &Document) -> Option<WithdrawalDoc> {
    let props = doc.properties();
    Some(WithdrawalDoc {
        id: doc.id(),
        owner: doc.owner_id(),
        amount: props.get("amount")?.to_integer::<u64>().ok()?,
        created_ms: doc.created_at(),
        script: props.get("outputScript")?.to_binary_bytes().ok()?,
    })
}

/// The document among `docs` whose payout goes to `script` (the unlock's output): one address
/// per script, and never another payout's.
pub fn of_payout(docs: &[WithdrawalDoc], script: &[u8]) -> Option<WithdrawalDoc> {
    docs.iter().find(|d| d.script == script).cloned()
}

/// Where `doc` came from: an identity of `identities`, the pool by one of `spent` (the
/// nullifiers of every spent note of the wallet's pool, not a spend's own list: the restore
/// links only the change, and the first action's note may be any of them), or Platform.
pub fn classify(doc: &WithdrawalDoc, identities: &[Identifier], spent: &[[u8; 32]]) -> Origin {
    if identities.contains(&doc.owner) {
        return Origin::Identity(doc.owner);
    }
    if doc.owner == withdrawals_contract::OWNER_ID
        && let Some(nf) = spent
            .iter()
            .find(|nf| pool_withdrawal_id(nf, &doc.script) == doc.id)
    {
        return Origin::Pool { nullifier: *nf };
    }
    Origin::Platform
}

impl crate::chain::Chain {
    /// BUG-275: the withdrawal document of the asset unlock with payload `index` paying
    /// `address` (one of the wallet's), read with proof — a status at a time, as Drive asks.
    /// `None`: no document of that index pays that address (yet). Where it came from is the
    /// caller's to say, by the notes it knows then ([`classify`], r1 Must 1).
    pub async fn payout_document(
        &self,
        index: u64,
        address: &str,
    ) -> Result<Option<WithdrawalDoc>, String> {
        use dash_sdk::platform::FetchMany;
        use dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
        use key_wallet::dashcore::address::NetworkUnchecked;
        self.ready()?;
        let sdk = self.sdk().await?;
        let withdrawals = load_system_data_contract(SystemDataContract::Withdrawals, sdk.version())
            .map(Arc::new)
            .map_err(|e| format!("{e:?}"))?;
        let script = address
            .parse::<key_wallet::dashcore::Address<NetworkUnchecked>>()
            .map_err(|e| e.to_string())?
            .require_network(crate::network::dash())
            .map_err(|e| e.to_string())?
            .script_pubkey()
            .to_bytes();
        for status in SENT {
            let docs: Vec<WithdrawalDoc> =
                Document::fetch_many(&sdk, by_index(withdrawals.clone(), status, index)?)
                    .await
                    .map_err(|e| format!("{e:?}"))?
                    .into_values()
                    .flatten()
                    .filter_map(|d| withdrawal_of(&d))
                    .collect();
            if let Some(doc) = of_payout(&docs, &script) {
                return Ok(Some(doc));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(owner: Identifier, id: Identifier, script: &[u8]) -> WithdrawalDoc {
        WithdrawalDoc {
            id,
            owner,
            amount: 1_990_000_000,
            created_ms: Some(1_763_800_000_000),
            script: script.to_vec(),
        }
    }

    /// The id by the transformer's formula, its entropy built by hand: the contract's id and
    /// owner, the type's name, and `nullifier ‖ script`.
    #[test]
    fn the_pool_withdrawal_id_is_drives() {
        let nf = [7u8; 32];
        let script = [0x76, 0xa9, 0x14, 1, 2, 3];
        let mut entropy = nf.to_vec();
        entropy.extend_from_slice(&script);
        let by_hand = Document::generate_document_id_v0(
            &withdrawals_contract::ID,
            &withdrawals_contract::OWNER_ID,
            "withdrawal",
            &entropy,
        );
        assert_eq!(pool_withdrawal_id(&nf, &script), by_hand);
    }

    /// The architect's class tests: a note's nullifier anywhere among our spent ones finds the
    /// pool; a dummy first action's (not ours) gives «from Platform», not «pool»; an identity's
    /// document is the identity's; another output script is no link.
    #[test]
    fn only_a_proved_origin_is_named() {
        let script = vec![0x76, 0xa9, 0x14, 9, 9];
        let ours = [[1u8; 32], [2u8; 32], [3u8; 32]];
        // The pool, by the third of our spent notes (not the first, not a spend's own list).
        let pooled = doc(
            withdrawals_contract::OWNER_ID,
            pool_withdrawal_id(&ours[2], &script),
            &script,
        );
        assert_eq!(
            classify(&pooled, &[], &ours),
            Origin::Pool { nullifier: ours[2] }
        );
        // A dummy first action: its nullifier is none of ours — «from Platform».
        let dummy = doc(
            withdrawals_contract::OWNER_ID,
            pool_withdrawal_id(&[42u8; 32], &script),
            &script,
        );
        assert_eq!(classify(&dummy, &[], &ours), Origin::Platform);
        // An identity's withdrawal.
        let me = Identifier::from([5u8; 32]);
        let credits = doc(me, Identifier::from([6u8; 32]), &script);
        assert_eq!(classify(&credits, &[me], &ours), Origin::Identity(me));
        // The unlock pays another script: no document of it.
        assert_eq!(
            of_payout(std::slice::from_ref(&pooled), &[0x76, 0xa9, 0x14, 0]),
            None
        );
        assert_eq!(
            of_payout(std::slice::from_ref(&pooled), &script),
            Some(pooled)
        );
    }
}
