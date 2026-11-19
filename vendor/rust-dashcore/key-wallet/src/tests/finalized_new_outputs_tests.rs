//! evolite BUG-232: a chain-locked transaction read again after one of its
//! outputs became ours (the address pool took the address in after the record
//! was made) gives that output its coin. Every other finalized transaction is
//! left as it was.

use dashcore::blockdata::transaction::{OutPoint, Transaction};
use dashcore::hashes::Hash;
use dashcore::{Address, BlockHash, TxIn, TxOut, Txid};

use crate::managed_account::address_pool::{AddressPoolType, KeySource};
use crate::managed_account::managed_account_trait::ManagedAccountTrait;
use crate::test_utils::TestWalletContext;
use crate::transaction_checking::{BlockInfo, TransactionContext};
use crate::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;

const FUNDS: u64 = 1_000_000;
const CHANGE: u64 = 980_000;

fn chain_locked(height: u32) -> TransactionContext {
    TransactionContext::InChainLockedBlock(BlockInfo::new(
        height,
        BlockHash::from_byte_array([height as u8; 32]),
        1_700_000_000,
    ))
}

fn spending(from: OutPoint, to: &[(&Address, u64)]) -> Transaction {
    Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn {
            previous_output: from,
            ..TxIn::default()
        }],
        output: to
            .iter()
            .map(|(address, value)| TxOut {
                value: *value,
                script_pubkey: address.script_pubkey(),
            })
            .collect(),
        special_transaction_payload: None,
    }
}

/// The BIP44 account 0's change address at `index`, made apart from the wallet.
fn change_at(ctx: &TestWalletContext, index: u32) -> Address {
    let mut pool = internal_pool(ctx).clone();
    pool.gap_limit = index + 1;
    pool.maintain_gap_limit(&KeySource::Public(ctx.xpub)).unwrap();
    pool.address_at_index(index).unwrap()
}

fn internal_pool(ctx: &TestWalletContext) -> &crate::managed_account::address_pool::AddressPool {
    ctx.bip44_account()
        .managed_account_type()
        .address_pools()
        .into_iter()
        .find(|p| p.pool_type == AddressPoolType::Internal)
        .unwrap()
}

/// The change pool made up to `index`, as a later pool (or a restart's longer
/// one) would have it, the address not marked used.
fn take_in(ctx: &mut TestWalletContext, index: u32) {
    let source = KeySource::Public(ctx.xpub);
    for pool in ctx
        .managed_wallet
        .first_bip44_managed_account_mut()
        .unwrap()
        .managed_account_type_mut()
        .address_pools_mut()
    {
        if pool.pool_type == AddressPoolType::Internal {
            let gap = pool.gap_limit;
            pool.gap_limit = index + 1;
            pool.maintain_gap_limit(&source).unwrap();
            pool.gap_limit = gap;
        }
    }
}

struct Paid {
    ctx: TestWalletContext,
    tx1: Transaction,
    change_index: u32,
}

/// A coin of ours (block 10), then tx1 spends it: 10 000 to another wallet and
/// the change 3 past the change pool (block 20), both chain-locked. The change
/// is not ours when tx1 is read.
async fn paid() -> Paid {
    let mut ctx = TestWalletContext::new_random();
    let other = TestWalletContext::new_random().receive_address;
    let funding = spending(
        OutPoint::new(Txid::from_byte_array([50; 32]), 0),
        &[(&ctx.receive_address.clone(), FUNDS)],
    );
    let change_index = internal_pool(&ctx).highest_generated.unwrap() + 3;
    let change = change_at(&ctx, change_index);
    let tx1 = spending(OutPoint::new(funding.txid(), 0), &[(&other, 10_000), (&change, CHANGE)]);
    ctx.check_transaction(&funding, chain_locked(10)).await;
    ctx.check_transaction(&tx1, chain_locked(20)).await;
    assert_eq!(ctx.managed_wallet.balance().total(), 0, "the change is not ours yet");
    assert!(ctx.bip44_account().transaction_is_finalized(&tx1.txid()));
    Paid {
        ctx,
        tx1,
        change_index,
    }
}

#[tokio::test]
async fn a_finalized_tx_read_again_credits_an_output_that_became_ours() {
    let Paid {
        mut ctx,
        tx1,
        change_index,
    } = paid().await;
    take_in(&mut ctx, change_index);
    let result = ctx.check_transaction(&tx1, chain_locked(20)).await;
    let coin = OutPoint::new(tx1.txid(), 1);
    let utxo = ctx.bip44_account().utxos.get(&coin).expect("the change's coin");
    assert!(utxo.is_confirmed);
    assert_eq!(utxo.height, 20);
    assert_eq!(ctx.managed_wallet.balance().total(), CHANGE);
    assert!(ctx.bip44_account().transaction_is_finalized(&tx1.txid()));
    #[cfg(feature = "keep-finalized-transactions")]
    assert!(result.updated_records.iter().any(|r| r.txid == tx1.txid()), "the caller is told");
    #[cfg(not(feature = "keep-finalized-transactions"))]
    let _ = result;
    // Read once more: nothing new.
    let again = ctx.check_transaction(&tx1, chain_locked(20)).await;
    assert!(again.updated_records.is_empty());
    assert_eq!(ctx.managed_wallet.balance().total(), CHANGE);
}

#[tokio::test]
async fn an_output_spent_by_a_transaction_read_first_gets_no_coin() {
    let Paid {
        mut ctx,
        tx1,
        change_index,
    } = paid().await;
    // tx2 spends the change back to our receiving address: read (its output is ours).
    let back = ctx.receive_address.clone();
    let tx2 = spending(OutPoint::new(tx1.txid(), 1), &[(&back, CHANGE - 300)]);
    ctx.check_transaction(&tx2, chain_locked(30)).await;
    take_in(&mut ctx, change_index);
    let result = ctx.check_transaction(&tx1, chain_locked(20)).await;
    // tx2's input is attributed to the change now (`attribute_late_inputs`); tx1 itself
    // gives no coin.
    assert!(!result.updated_records.iter().any(|r| r.txid == tx1.txid()));
    assert!(!ctx.bip44_account().utxos.contains_key(&OutPoint::new(tx1.txid(), 1)));
    assert_eq!(ctx.managed_wallet.balance().total(), CHANGE - 300);
}

#[tokio::test]
async fn a_finalized_tx_with_nothing_new_is_left_as_it_was() {
    let Paid {
        mut ctx,
        tx1,
        ..
    } = paid().await;
    let revision = ctx.bip44_account().monitor_revision();
    let result = ctx.check_transaction(&tx1, chain_locked(20)).await;
    assert!(result.updated_records.is_empty());
    assert!(result.new_records.is_empty());
    assert_eq!(ctx.bip44_account().monitor_revision(), revision);
    assert_eq!(ctx.managed_wallet.balance().total(), 0);
}
