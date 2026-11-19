//! BUG-222 r2: every query of ours on a system contract is one Drive takes: an index of the
//! contract serves it. Checked offline with Drive's own choice of index, as the node makes it;
//! a query no index serves is refused there every time (`InvalidArgument`), whatever the node.

use std::sync::Arc;

use dash_sdk::drive::query::DriveDocumentQuery;
use dash_sdk::platform::{DocumentQuery, Identifier};
use dpp::data_contract::DataContract;
use dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dpp::version::PlatformVersion;

use super::token_actions::claim_query;
use super::token_manage::{IssuerDocKind, freeze_query, issuer_doc_query};
use crate::platform::{platform_history, token_history};

fn contract(which: SystemDataContract) -> Arc<DataContract> {
    Arc::new(load_system_data_contract(which, PlatformVersion::latest()).expect("the contract"))
}

/// The index Drive picks for `query`, or why none serves it. Then the path query Drive builds
/// from it, as the node does before it runs one: that checks the conditions' shapes and the
/// range and order combinations, which the choice of index does not (review r2).
fn served(query: &DocumentQuery) -> Result<String, String> {
    let drive = DriveDocumentQuery::try_from(query).map_err(|e| format!("{e:?}"))?;
    let index = drive
        .find_best_index(PlatformVersion::latest())
        .map(|index| index.name.clone())
        .map_err(|e| format!("{e:?}"))?;
    drive
        .construct_path_query(None, PlatformVersion::latest())
        .map_err(|e| format!("{e:?}"))?;
    Ok(index)
}

fn ok(what: &str, query: Result<DocumentQuery, String>) -> String {
    let query = query.unwrap_or_else(|e| panic!("{what}: {e}"));
    served(&query).unwrap_or_else(|e| panic!("{what}: no index serves it: {e}"))
}

const TOKEN: Identifier = Identifier::new([7; 32]);
const ME: Identifier = Identifier::new([9; 32]);

#[test]
fn the_token_history_queries_are_served_by_an_index() {
    let history = contract(SystemDataContract::TokenHistory);
    for kind in IssuerDocKind::ALL {
        ok(
            &format!("{kind:?}"),
            issuer_doc_query(history.clone(), kind, TOKEN, ME),
        );
    }
    // The freezes, the first page and a next one, by whom they froze.
    for after in [None, Some(ME)] {
        assert_eq!(
            ok("freeze", freeze_query(history.clone(), TOKEN, after)),
            "byFrozenIdentityId"
        );
    }
    for after in [None, Some(1_763_300_000_000)] {
        ok("claim", claim_query(history.clone(), TOKEN, ME, after));
    }
    use token_history::{Direction, Side};
    for side in [Side::From, Side::To] {
        for dir in [Direction::Newer, Direction::Older] {
            for at in [None, Some(1_763_300_000_000)] {
                ok(
                    "transfer",
                    token_history::transfer_query(history.clone(), TOKEN, ME, side, dir, at),
                );
            }
        }
    }
}

/// The query of before BUG-222 r2: the freezes by date alone. No index of `freeze` starts with
/// `tokenId` then `$createdAt`: refused, as testnet refused it on every token page.
#[test]
fn the_freezes_by_date_alone_are_refused() {
    use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
    let history = contract(SystemDataContract::TokenHistory);
    let by_date = DocumentQuery::new(history, "freeze")
        .unwrap()
        .with_where(WhereClause {
            field: "tokenId".into(),
            operator: WhereOperator::Equal,
            value: dpp::platform_value::Value::Identifier(TOKEN.to_buffer()),
        })
        .with_order_by(OrderClause {
            field: "$createdAt".into(),
            ascending: true,
        })
        .with_limit(100);
    assert!(served(&by_date).is_err(), "{:?}", served(&by_date));
}

#[test]
fn the_dashpay_dpns_and_withdrawal_queries_are_served_by_an_index() {
    let dashpay = contract(SystemDataContract::Dashpay);
    for field in ["$ownerId", "toUserId"] {
        ok(
            &format!("contactRequest by {field}"),
            super::contacts::contact_request_query(dashpay.clone(), field, ME),
        );
    }
    // FEAT-0020-resend: our requests to one contact, by the pair.
    assert_eq!(
        ok(
            "contactRequest by the pair",
            super::contacts::sent_to_query(dashpay.clone(), ME, TOKEN),
        ),
        "ownerIdUserId"
    );
    ok(
        "profile",
        platform_history::query(&dashpay, "profile", "$ownerId", ME, None),
    );
    let dpns = contract(SystemDataContract::DPNS);
    ok(
        "domain",
        platform_history::query(&dpns, "domain", "records.identity", ME, None),
    );
    let withdrawals = contract(SystemDataContract::Withdrawals);
    ok(
        "withdrawal",
        platform_history::query(&withdrawals, "withdrawal", "$ownerId", ME, Some(1)),
    );
}
