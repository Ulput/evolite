//! What a series takes and costs, without the network (FEAT-0038 R-6, R-7, R-10): the serials
//! from instances read with proof, page by page by `byImageSerial`, and one write's estimate.

use dpp::prelude::Identifier;

/// One instance as a page of `byImageSerial` shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance {
    pub id: Identifier,
    pub serial: u32,
    pub owner: Identifier,
    /// `$price` in credits; `None`: not on sale.
    pub price: Option<u64>,
}

/// Pages read before giving up on finding enough (architect, 2026-11-20): «try fewer».
pub const MAX_PAGES: usize = 20;

/// Which instances of the owner a series takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Held and not on sale: price, gift, delete — read by serial ascending.
    Free,
    /// On sale at this price, or at any — take off sale: read by serial descending.
    OnSale(Option<u64>),
}

impl Pick {
    /// Whether the pages are read from the highest serial down.
    pub fn descending(self) -> bool {
        matches!(self, Pick::OnSale(_))
    }

    fn takes(self, i: &Instance, me: &Identifier) -> bool {
        i.owner == *me
            && match self {
                Pick::Free => i.price.is_none(),
                Pick::OnSale(None) => i.price.is_some(),
                Pick::OnSale(Some(p)) => i.price == Some(p),
            }
    }
}

/// The instances `pick` takes for `me` from a page read in the order `pick` asks, until `want`
/// are taken: the ones taken so far are in `taken`. Returns whether `want` is reached.
pub fn take_from_page(
    page: &[Instance],
    pick: Pick,
    me: &Identifier,
    want: usize,
    taken: &mut Vec<Instance>,
) -> bool {
    for i in page {
        if taken.len() >= want {
            break;
        }
        if pick.takes(i, me) {
            taken.push(*i);
        }
    }
    taken.len() >= want
}

/// The serials a mint takes: after the highest ever issued, from the wallet's journal or proved
/// in the network, whichever is higher (R-6); never past `supply`. `Err(left)`: only `left`
/// more can be issued.
pub fn next_serials(
    journal_max: Option<u32>,
    network_max: Option<u32>,
    count: u32,
    supply: u32,
) -> Result<Vec<u32>, u32> {
    // default-ok: none issued yet: serials start after 0
    let after = journal_max.max(network_max).unwrap_or(0);
    let left = supply.saturating_sub(after);
    if count > left {
        return Err(left);
    }
    Ok((after + 1..=after + count).collect())
}

/// What one write of a series is estimated at, in credits: the probe's measure on testnet
/// (2026-11-19) and a third on top (AC-T-M4), never below what the network takes.
pub mod per_write {
    // Not below the probe's measures and a third (checked when it builds). BUG-259 (the
    // architect): the margin stays — the windows say «at most» — but the first instance of a
    // contract (its document type's trees made) is its own; when the network's fact is again
    // more than a tenth below a measure, the measure is updated, not the margin.
    const _: () = assert!(MINT >= 38_869_000 * 4 / 3);
    const _: () = assert!(MINT_FIRST >= 65_053_000 * 4 / 3);
    const _: () = assert!(MINT_FIRST > MINT);
    const _: () = assert!(PRICE >= 3_981_000 * 4 / 3);
    const _: () = assert!(TRANSFER >= 108_447_000 * 4 / 3);

    /// An instance: 0.00038869.
    pub const MINT: u64 = 52_000_000;
    /// The first instance in a contract: 0.00065053, its trees made.
    pub const MINT_FIRST: u64 = 87_000_000;

    /// `count` instances issued, the first of them the contract's first when `first` (no
    /// instance on the network yet, `nft_max_serial` = `None`).
    pub const fn mint_total(first: bool, count: u64) -> u64 {
        if first && count > 0 {
            MINT_FIRST + MINT * (count - 1)
        } else {
            MINT * count
        }
    }
    /// A price, or a price over a price: 0.00003981.
    pub const PRICE: u64 = 5_400_000;
    /// A transfer, to another (a gift) or to oneself (off sale): 0.00108447.
    pub const TRANSFER: u64 = 145_000_000;
    /// A model's «image» document, a line in its stored bytes (BUG-343), over both protocols the
    /// build talks to (BUG-259 r2, the architect: a measure is the higher of the two):
    /// - **14**: the probe's 5 000 bytes took 0.0014857, QA's 446 bytes 0.00031902 (2026-11-27)
    ///   — ≈ 25 620 credits a byte over a base of ≈ 20 480 000;
    /// - **15** (PV15, 2026-11-27): 4 930 bytes 0.00146794, 106 bytes 0.00014328 — ≈ 27 460
    ///   credits a byte over a base of ≈ 11 420 000: dearer a byte, so a large picture costs
    ///   more at 15.
    ///
    /// The estimate: the base of 14, the slope of 15, a third on top of both — down to its largest
    /// write ([`PARTS_TOTAL`](super::super::image::PARTS_TOTAL)). The base alone was 50 000 000
    /// before, more than a small picture took in all (an estimate twice the fact).
    pub const IMAGE_BASE: u64 = 28_000_000;
    pub const IMAGE_PER_BYTE: u64 = 36_700;
    /// The network's charge for `bytes` on the line through two measures `(bytes, credits)`.
    const fn on_line(bytes: u64, from: (u64, u64), to: (u64, u64)) -> u64 {
        from.1 + (bytes - from.0) * (to.1 - from.1) / (to.0 - from.0)
    }
    const AT_14: [(u64, u64); 2] = [(446, 31_902_000), (5_000, 148_570_000)];
    const AT_15: [(u64, u64); 2] = [(106, 14_328_000), (4_930, 146_794_000)];
    const LARGEST: u64 = super::super::image::PARTS_TOTAL as u64;
    // Each measure and the largest picture on each protocol's line, with its third.
    const _: () = assert!(image(5_000) >= 148_570_000 * 4 / 3);
    const _: () = assert!(image(446) >= 31_902_000 * 4 / 3);
    const _: () = assert!(image(4_930) >= 146_794_000 * 4 / 3);
    const _: () = assert!(image(106) >= 14_328_000 * 4 / 3);
    const _: () = assert!(image(LARGEST as usize) >= on_line(LARGEST, AT_14[0], AT_14[1]) * 4 / 3);
    const _: () = assert!(image(LARGEST as usize) >= on_line(LARGEST, AT_15[0], AT_15[1]) * 4 / 3);

    /// The «image» write of a picture stored in `bytes`.
    pub const fn image(bytes: usize) -> u64 {
        IMAGE_BASE + IMAGE_PER_BYTE * bytes as u64
    }

    /// A delete: the network refunded more than it took (−0.00027055); shown as at most one
    /// write, with no promise of a refund (R-12, architect 2026-11-20).
    pub const DELETE: u64 = TRANSFER;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> Identifier {
        Identifier::from([n; 32])
    }

    fn page(rows: &[(u32, u8, Option<u64>)]) -> Vec<Instance> {
        rows.iter()
            .map(|&(serial, owner, price)| Instance {
                id: Identifier::from([serial as u8; 32]),
                serial,
                owner: id(owner),
                price,
            })
            .collect()
    }

    /// Free ones of mine, lowest first, others' and priced ones passed over, across pages.
    #[test]
    fn free_ones_are_mine_without_a_price_lowest_first() {
        let me = id(1);
        let first = page(&[(1, 2, None), (2, 1, Some(5)), (3, 1, None), (4, 3, None)]);
        let second = page(&[(5, 1, None), (6, 1, None), (7, 1, None)]);
        let mut taken = Vec::new();
        assert!(!take_from_page(&first, Pick::Free, &me, 3, &mut taken));
        assert!(take_from_page(&second, Pick::Free, &me, 3, &mut taken));
        let serials: Vec<u32> = taken.iter().map(|i| i.serial).collect();
        assert_eq!(serials, [3, 5, 6]);
    }

    /// Off sale: mine at that price, highest first (the pages are read descending).
    #[test]
    fn on_sale_ones_at_a_price_highest_first() {
        let me = id(1);
        let desc = page(&[
            (9, 1, Some(5)),
            (8, 1, Some(7)),
            (7, 2, Some(5)),
            (6, 1, Some(5)),
        ]);
        let mut taken = Vec::new();
        assert!(Pick::OnSale(Some(5)).descending());
        assert!(take_from_page(
            &desc,
            Pick::OnSale(Some(5)),
            &me,
            2,
            &mut taken
        ));
        let serials: Vec<u32> = taken.iter().map(|i| i.serial).collect();
        assert_eq!(serials, [9, 6]);
        let mut any = Vec::new();
        take_from_page(&desc, Pick::OnSale(None), &me, 9, &mut any);
        assert_eq!(any.len(), 3, "mine on sale at any price");
    }

    /// R-6: after the highest of the journal and the network; never past the supply.
    #[test]
    fn mint_serials_follow_the_highest_ever_issued() {
        assert_eq!(next_serials(None, None, 3, 10), Ok(vec![1, 2, 3]));
        // The highest deleted: the journal remembers it, the network does not.
        assert_eq!(next_serials(Some(10), Some(9), 2, 20), Ok(vec![11, 12]));
        // No journal (restored from the phrase): after the network's.
        assert_eq!(next_serials(None, Some(9), 1, 20), Ok(vec![10]));
        assert_eq!(next_serials(Some(8), None, 3, 10), Err(2));
        assert_eq!(next_serials(Some(10), None, 1, 10), Err(0));
    }

    /// BUG-343: a picture's estimate is the measures' line and a third — at both sizes the
    /// network was measured at, never below its fact and never half over it. r2 (PV15): at 15's
    /// sizes too, never below — the bound of a half is 14's (15's base is lower).
    #[test]
    fn a_picture_s_estimate_follows_both_measures() {
        use super::per_write::image;
        // BUG-343: at most a half over the network's fact at both sizes measured, never below.
        for (bytes, fact) in [(446, 31_902_000u64), (5_000, 148_570_000)] {
            let estimate = image(bytes);
            assert!(
                estimate >= fact * 4 / 3,
                "{bytes}: {estimate} under the fact and a third"
            );
            assert!(
                estimate <= fact * 3 / 2,
                "{bytes}: {estimate} over half above the fact"
            );
        }
        for (bytes, fact) in [(106, 14_328_000u64), (4_930, 146_794_000)] {
            let estimate = image(bytes);
            assert!(
                estimate >= fact * 4 / 3,
                "{bytes} at 15: {estimate} under the fact and a third"
            );
        }
    }

    /// BUG-259 (the architect): a model with no instance yet — issuing 3 is the contract's first
    /// and two more; 3 more after them are three of the usual.
    #[test]
    fn the_first_instance_of_a_contract_costs_its_own() {
        use super::per_write::{MINT, MINT_FIRST, mint_total};
        assert_eq!(mint_total(true, 3), MINT_FIRST + 2 * MINT);
        assert_eq!(mint_total(false, 3), 3 * MINT);
        assert_eq!(mint_total(true, 0), 0, "none issued, none paid");
    }
}
