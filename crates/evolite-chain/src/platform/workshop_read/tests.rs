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

/// A `purchase` document read into a sale: buyer is its owner; its price, kept in credits, in
/// duffs (BUG-276 r1: 100 000 000 000 credits are 1 DASH, not 1 000).
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
            ("price".to_owned(), Value::U64(100_000_000_000)),
        ]
        .into(),
        created_at: Some(1_700_000_000_000),
        ..Default::default()
    });
    let s = sale_of(&doc).unwrap();
    assert_eq!(s.buyer, id(8).to_string(Encoding::Base58));
    assert_eq!(s.contract, id(1).to_string(Encoding::Base58));
    assert_eq!(s.instance, id(2).to_string(Encoding::Base58));
    assert_eq!((s.price, s.at_ms), (100_000_000, 1_700_000_000_000));
}

/// A model document with its picture in `part0…` (or one `image`) and its `imageType`.
fn model_doc(n: u32, image_type: u8, parts: &[&[u8]]) -> Document {
    let mut properties: std::collections::BTreeMap<String, Value> = [
        ("name".to_owned(), Value::Text("Winter tram".into())),
        ("imageType".to_owned(), Value::U8(image_type)),
        ("supply".to_owned(), Value::U32(100)),
    ]
    .into();
    if let [one] = parts {
        properties.insert("image".into(), Value::Bytes(one.to_vec()));
    } else {
        for (i, part) in parts.iter().enumerate() {
            properties.insert(format!("part{i}"), Value::Bytes(part.to_vec()));
        }
    }
    Document::V0(DocumentV0 {
        id: id(n),
        properties,
        ..Default::default()
    })
}

/// BUG-269: a JPEG model stored in three parts reads back as one picture — the parts joined in
/// their order, plain, under the key the model is shown by — with nothing kept from its making.
#[test]
fn a_multi_part_jpeg_model_reads_back_as_its_picture() {
    let jpeg: Vec<u8> = [0xFF, 0xD8, 0xFF, 0xE0]
        .into_iter()
        .chain((0..12_000u32).map(|i| (i % 251) as u8))
        .collect();
    let parts: Vec<&[u8]> = jpeg.chunks(5_120).collect();
    assert_eq!(parts.len(), 3);
    let doc = model_doc(1, 3, &parts);
    let (key, bytes, deflated) = model_picture(&doc).expect("a picture");
    assert_eq!(bytes, jpeg, "the parts joined in order");
    assert!(!deflated, "a JPEG is stored as it is");
    assert_eq!(
        key,
        model_of(&doc).art,
        "drawn by the key the model is shown by"
    );
}

/// BUG-269: an SVG the Workshop deflated is told by its first bytes; one stored plain is not.
#[test]
fn a_deflated_svg_model_is_told_from_a_plain_one() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8"/></svg>"#;
    let packed = miniz_oxide::deflate::compress_to_vec(svg, 9);
    let (_, _, deflated) = model_picture(&model_doc(2, 0, &[&packed])).unwrap();
    assert!(deflated);
    let (_, _, plain) = model_picture(&model_doc(3, 0, &[svg])).unwrap();
    assert!(!plain);
    assert_eq!(model_picture(&model_doc(4, 3, &[])), None, "no picture");
}

/// BUG-271 (TEAM.md §8d, kind В1 — «works only where it was made»): a fresh data folder, with
/// no `contracts.db` from the copy that registered the collection, finds the identity's own
/// collection through its identity nonces — and keeps it, so the Workshop's list (which reads
/// the cache) shows it. A contract of the identity that is not of the NFT standard is kept but
/// is no collection; the nonce's high bits (missed revisions) are not candidates; a later pass
/// asks only from where the last one stopped.
#[test]
fn a_fresh_data_folder_finds_its_own_collections_by_nonce() {
    use crate::platform::contracts::ContractCache;
    use dpp::data_contract::accessors::v0::DataContractV0Setters;
    let dir = std::env::temp_dir().join(format!("evolite-own-contracts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cache = ContractCache::new(dir.join("contracts.db"));
    let owner = id(77);
    let at = |nonce| DataContract::generate_data_contract_id_v0(owner, nonce);
    let mut collection = super::profile::tests::contract(&super::profile::tests::schema(), true)
        .expect("the standard's schema");
    collection.set_id(at(3));
    collection.set_owner_id(owner);
    // Not read-only: not of the standard (a token contract or another of its own).
    let mut other = super::profile::tests::contract(&super::profile::tests::schema(), false)
        .expect("a contract of its own");
    other.set_id(at(5));
    other.set_owner_id(owner);
    let network = [collection.clone(), other.clone()];
    let asked = RefCell::new(Vec::new());
    let fetch = |ids: Vec<Identifier>| {
        asked.borrow_mut().push(ids.clone());
        let found: Vec<Arc<DataContract>> = network
            .iter()
            .filter(|c| ids.contains(&c.id()))
            .map(|c| Arc::new(c.clone()))
            .collect();
        std::future::ready(Ok(found))
    };
    // Nonce 6, with a missed-revision bit above the value's 40 bits.
    let nonce = 6 | (1u64 << 45);
    let (new, to) = run(scan_own(&cache, owner, 1, nonce, fetch)).unwrap();
    assert_eq!(
        (new, to),
        (1, 6),
        "one collection; read to the nonce's value"
    );
    assert_eq!(asked.borrow()[0], (1..=6).map(at).collect::<Vec<_>>());
    let listed: Vec<Identifier> = cache.nft_contracts().into_iter().map(|(i, _)| i).collect();
    assert_eq!(
        listed,
        vec![at(3)],
        "the collection; the other contract is no collection"
    );
    assert!(cache.get(&at(5)).is_some(), "kept all the same");
    // The next opening: from nonce 3 (the last three again), nothing new.
    asked.borrow_mut().clear();
    let fetch = |ids: Vec<Identifier>| {
        asked.borrow_mut().push(ids.clone());
        let found: Vec<Arc<DataContract>> = network
            .iter()
            .filter(|c| ids.contains(&c.id()))
            .map(|c| Arc::new(c.clone()))
            .collect();
        std::future::ready(Ok(found))
    };
    let (new, _) = run(scan_own(&cache, owner, 4, 6, fetch)).unwrap();
    assert_eq!(new, 0, "known already");
    assert_eq!(asked.borrow()[0], (4..=6).map(at).collect::<Vec<_>>());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A DocumentHistory `transfer` by `by` of `instance` to `to`, in `contract`'s type `kind`.
fn transfer(by: u32, contract: u32, kind: &str, instance: u32, to: u32, at: u64) -> Document {
    Document::V0(DocumentV0 {
        id: id(1_000 + instance),
        owner_id: id(by),
        properties: [
            (
                "dataContractId".to_owned(),
                Value::Identifier(id(contract).to_buffer()),
            ),
            ("documentTypeName".to_owned(), Value::Text(kind.into())),
            (
                "documentId".to_owned(),
                Value::Identifier(id(instance).to_buffer()),
            ),
            (
                "toIdentityId".to_owned(),
                Value::Identifier(id(to).to_buffer()),
            ),
        ]
        .into(),
        created_at: Some(at),
        ..Default::default()
    })
}

/// W7 «Given» (the architect's test): a gift counts; «take off sale» — a transfer to oneself —
/// does not, nor a holder's resale, nor another collection's or another type's transfer.
#[test]
fn only_a_gift_of_ones_own_copy_counts() {
    let (owner, collection) = (id(1), id(500));
    let gifts: Vec<GiftDoc> = [
        transfer(1, 500, "nft", 7, 2, 10),   // a gift to @bob
        transfer(1, 500, "nft", 8, 1, 11),   // «take off sale»: to oneself
        transfer(3, 500, "nft", 7, 4, 12),   // @bob resells it
        transfer(1, 501, "nft", 9, 2, 13),   // another collection
        transfer(1, 500, "image", 5, 2, 14), // another type
    ]
    .iter()
    .filter_map(|d| gift_of(d, collection, "nft", owner))
    .collect();
    assert_eq!(
        gifts,
        vec![GiftDoc {
            instance: id(7),
            to: id(2),
            at_ms: 10
        }]
    );
}

/// F10: in an open collection anyone may write a model and a copy of it. An NFT is the issuer's
/// (its picture drawn at once from a chosen collection, «issue» of the issuer) only when the
/// issuer wrote both: by the type in a closed collection, by `$creatorId` for a copy, by its
/// owner for a model that never changes hands. Another's is shown as its writer's, never the
/// issuer's; a copy whose maker Platform did not keep is not the issuer's.
#[test]
fn only_what_the_issuer_wrote_is_the_issuers() {
    use dpp::data_contract::accessors::v0::DataContractV0Setters;
    use serde_json::json;
    let (issuer, stranger, holder) = (id(1), id(2), id(3));
    let collection = |open: bool| {
        let mut s = super::profile::tests::schema();
        if open {
            s["nft"]["creationRestrictionMode"] = json!(0);
            s["image"]["creationRestrictionMode"] = json!(0);
        }
        let mut c = super::profile::tests::contract(&s, true).expect("the standard's schema");
        c.set_owner_id(issuer);
        c
    };
    // A copy `holder` holds, made by `maker` (`None`: Platform kept no `$creatorId`).
    let copy = |maker: Option<Identifier>| {
        Document::V0(DocumentV0 {
            id: id(10),
            owner_id: holder,
            creator_id: maker,
            ..Default::default()
        })
    };
    // A model `by` wrote: it never changes hands, so its owner is its writer.
    let model = |by: Identifier| {
        Document::V0(DocumentV0 {
            id: id(20),
            owner_id: by,
            ..Default::default()
        })
    };
    let shown = |c: &DataContract, copy: &Document, model: &Document| {
        let writers = [
            super::chain::writer_of(c, "nft", copy),
            super::chain::writer_of(c, "image", model),
        ];
        super::chain::issuer_of(issuer, writers)
    };
    let issuers = (issuer.to_string(Encoding::Base58), true);
    let strangers = (stranger.to_string(Encoding::Base58), false);

    // Closed (the Workshop's own): only the issuer can write either, kept maker or not.
    let closed = collection(false);
    assert_eq!(shown(&closed, &copy(None), &model(issuer)), issuers);

    let open = collection(true);
    assert_eq!(shown(&open, &copy(Some(issuer)), &model(issuer)), issuers);
    assert_eq!(
        shown(&open, &copy(Some(stranger)), &model(stranger)),
        strangers,
        "a stranger's picture in the issuer's collection"
    );
    assert_eq!(
        shown(&open, &copy(Some(stranger)), &model(issuer)),
        strangers,
        "a stranger's copy of the issuer's model"
    );
    assert_eq!(
        shown(&open, &copy(Some(issuer)), &model(stranger)),
        strangers,
        "the issuer's copy of a stranger's model"
    );
    assert_eq!(
        shown(&open, &copy(None), &model(issuer)),
        (String::new(), false),
        "who made the copy is not known: not the issuer's"
    );
}
