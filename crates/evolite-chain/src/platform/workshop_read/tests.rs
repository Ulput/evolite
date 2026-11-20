use super::*;
use dpp::document::DocumentV0;
use std::cell::RefCell;

fn id(n: u32) -> Identifier {
    let mut b = [0u8; 32];
    b[28..].copy_from_slice(&n.to_be_bytes());
    Identifier::from(b)
}

/// Instance `n` with `price` (0: not for sale).
fn instance(n: u32, price: u64) -> Document {
    Document::V0(DocumentV0 {
        id: id(n),
        properties: [
            ("serial".to_owned(), Value::U32(n)),
            ("$price".to_owned(), Value::U64(price)),
        ]
        .into(),
        ..Default::default()
    })
}

/// A network of `total` instances, every `sale`-th one for sale; it answers pages after an id,
/// in its order, and counts its requests.
fn network(total: u32, sale: u32) -> (Vec<Document>, RefCell<u32>) {
    let docs = (1..=total)
        .map(|n| instance(n, if n % sale == 0 { 10 } else { 0 }))
        .collect();
    (docs, RefCell::new(0))
}

fn page_of(
    docs: &[Document],
    asked: &RefCell<u32>,
    after: Option<Identifier>,
) -> std::future::Ready<Result<Vec<Document>, String>> {
    *asked.borrow_mut() += 1;
    let page: Vec<Document> = docs
        .iter()
        .skip_while(|d| after.is_some_and(|a| d.id() != a))
        .skip(usize::from(after.is_some()))
        .take(PAGE as usize)
        .cloned()
        .collect();
    std::future::ready(Ok(page))
}

fn run<T>(f: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

/// AC-T-M5 after the probe: «for sale» is a filter after reading; pages go on until the screen's
/// page is full, at most 5 per action, then «Показать ещё» goes on from where it stopped.
#[test]
fn for_sale_reads_on_until_full_or_five_pages() {
    // One in 50 for sale: 2 per page of 100, 10 found in 5 pages of 1 000.
    let (docs, asked) = network(1000, 50);
    let page = run(pages_with(
        None,
        20,
        MAX_PAGES,
        |d| price_of(d).is_some(),
        |a| page_of(&docs, &asked, a),
    ))
    .unwrap();
    assert_eq!(*asked.borrow(), 5, "at most five pages per action");
    assert_eq!(page.read, 500);
    assert_eq!(page.docs.len(), 10);
    assert_eq!(
        page.next,
        Some(id(500)),
        "«Показать ещё» goes on after the last read"
    );
    let more = run(pages_with(
        page.next,
        20,
        MAX_PAGES,
        |d| price_of(d).is_some(),
        |a| page_of(&docs, &asked, a),
    ))
    .unwrap();
    assert_eq!(serial_of(&more.docs[0]), Some(550));
    // The last page was full: its end is known only from the next, empty one.
    assert_eq!(more.next, Some(id(1000)));
    let end = run(pages_with(
        more.next,
        20,
        MAX_PAGES,
        |d| price_of(d).is_some(),
        |a| page_of(&docs, &asked, a),
    ))
    .unwrap();
    assert!(
        end.docs.is_empty() && end.next.is_none(),
        "the collection's end"
    );
    assert_eq!(more.docs.len(), 10);

    // Enough found on the first page: one request.
    let (docs, asked) = network(1000, 2);
    let page = run(pages_with(
        None,
        20,
        MAX_PAGES,
        |d| price_of(d).is_some(),
        |a| page_of(&docs, &asked, a),
    ))
    .unwrap();
    assert_eq!((*asked.borrow(), page.docs.len()), (1, 50));
}

/// The architect's Q1: one's own instances page by `$id` with StartAfter, every one once.
#[test]
fn own_instances_page_by_id_each_once() {
    let (docs, asked) = network(250, 1);
    let mut seen = Vec::new();
    let mut next = None;
    loop {
        let page = run(pages_with(
            next,
            100,
            1,
            |_| true,
            |a| page_of(&docs, &asked, a),
        ))
        .unwrap();
        seen.extend(page.docs.iter().filter_map(serial_of));
        next = page.next;
        if next.is_none() {
            break;
        }
    }
    assert_eq!(seen, (1..=250).collect::<Vec<_>>());
    assert_eq!(*asked.borrow(), 3);
}

/// Any order the query asks (the series core: by serial, descending) is read through each once.
#[test]
fn a_descending_order_is_read_through_once() {
    let (mut docs, asked) = network(250, 1);
    docs.reverse();
    let mut seen = Vec::new();
    let mut next = None;
    loop {
        let page = run(pages_with(
            next,
            100,
            1,
            |_| true,
            |a| page_of(&docs, &asked, a),
        ))
        .unwrap();
        seen.extend(page.docs.iter().filter_map(serial_of));
        next = page.next;
        if next.is_none() {
            break;
        }
    }
    assert_eq!(seen, (1..=250).rev().collect::<Vec<_>>());
}

/// `$price` 0 is not for sale.
#[test]
fn price_zero_is_not_for_sale() {
    assert_eq!(price_of(&instance(1, 0)), None);
    assert_eq!(price_of(&instance(1, 7)), Some(7));
}

fn sale(n: u8, contract: &str, at_ms: u64) -> SaleDoc {
    SaleDoc {
        id: format!("p{n}"),
        contract: contract.into(),
        instance: format!("i{n}"),
        buyer: "b".into(),
        price: 100,
        at_ms,
    }
}

/// AC-T-W4: a sale is a line once, keyed by its `purchase`; reading again adds nothing, a new
/// one in the same millisecond is still new; a contract that does not fit the profile is passed
/// over.
#[test]
fn a_sale_is_read_once() {
    // "c" fits the profile, "x" is no NFT collection.
    let fits = |c: &str| Some(c == "c");
    let first = vec![sale(1, "c", 10), sale(2, "c", 20), sale(3, "x", 20)];
    let (sales, cursor) = new_sales(first.clone(), &SalesCursor::default(), fits);
    assert_eq!(
        sales.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["p1", "p2"]
    );
    let (again, same) = new_sales(first.clone(), &cursor, fits);
    assert!(again.is_empty(), "read again: nothing new");
    assert_eq!(same, cursor);
    let mut later = first;
    later.push(sale(4, "c", 20));
    let (new, _) = new_sales(later, &cursor, fits);
    assert_eq!(new, vec![sale(4, "c", 20)]);
}

/// Review r1 Should, case 1: a resale of an instance of someone else's collection is a sale
/// too: kept by the profile, not by «my collections».
#[test]
fn a_resale_of_another_collection_is_a_line() {
    // "theirs" is not the seller's own collection, but it fits the profile.
    let (sales, _) = new_sales(vec![sale(1, "theirs", 10)], &SalesCursor::default(), |_| {
        Some(true)
    });
    assert_eq!(sales, vec![sale(1, "theirs", 10)]);
}

/// Review r1 Should, case 2: a sale of a contract not read yet (not in the cache, the read
/// failed) stops the cursor before it: it is neither kept nor passed, and comes once the
/// contract is read.
#[test]
fn a_sale_of_a_contract_not_read_waits_for_it() {
    let docs = vec![sale(1, "c", 10), sale(2, "new", 20), sale(3, "c", 30)];
    let (sales, cursor) = new_sales(docs.clone(), &SalesCursor::default(), |c| {
        (c == "c").then_some(true)
    });
    assert_eq!(sales, vec![sale(1, "c", 10)], "only what comes before it");
    assert_eq!(cursor.at_ms, 10, "the cursor stops before the unread one");
    let (later, after) = new_sales(docs, &cursor, |_| Some(true));
    assert_eq!(later, vec![sale(2, "new", 20), sale(3, "c", 30)]);
    assert_eq!(after.at_ms, 30);
}

/// A `purchase` document read into a sale: buyer is its owner.
#[test]
fn a_purchase_document_is_a_sale() {
    let doc = Document::V0(DocumentV0 {
        id: id(7),
        owner_id: id(8),
        properties: [
            (
                "dataContractId".to_owned(),
                Value::Identifier(id(1).to_buffer()),
            ),
            ("documentTypeName".to_owned(), Value::Text("nft".into())),
            (
                "documentId".to_owned(),
                Value::Identifier(id(2).to_buffer()),
            ),
            ("price".to_owned(), Value::U64(5_000)),
        ]
        .into(),
        created_at: Some(1_700_000_000_000),
        ..Default::default()
    });
    let s = sale_of(&doc).unwrap();
    assert_eq!(s.buyer, id(8).to_string(Encoding::Base58));
    assert_eq!(s.contract, id(1).to_string(Encoding::Base58));
    assert_eq!(s.instance, id(2).to_string(Encoding::Base58));
    assert_eq!((s.price, s.at_ms), (5_000, 1_700_000_000_000));
}
