//! BUG-319: what the network really took to create an identity from the pool, against the
//! formula's floor — the last [`KEPT`] ratios, one file per network beside the wallets (a
//! property of the network, not of a wallet: not in a wallet's folder, not in its backup; the
//! next creation measures it again). The fee shown is an upper estimate from them
//! ([`upper`]): Drive adds the new identity's processing and storage, which grow with the
//! network's state and cannot be known beforehand.

use evolite_api::Duffs;
use std::path::PathBuf;

/// How many of the latest measurements are kept.
pub const KEPT: usize = 10;
/// With nothing measured yet: the floor × 2.5.
pub const UNMEASURED_PERMILLE: u64 = 2_500;
/// Above the highest ratio measured: × 1.15.
pub const MARGIN_PERCENT: u64 = 115;

/// The fee shown before a creation (R-2 of BUG-319): never below what was measured —
/// `floor` × the highest of `ratios` (in thousandths) × 1.15, or × 2.5 with none. Rounded up:
/// a fee is never shown smaller than it is.
pub fn upper(floor: Duffs, ratios: &[u32]) -> Duffs {
    let floor = floor.max(0) as u128;
    let scaled = match ratios.iter().max() {
        Some(&max) => floor * max as u128 * MARGIN_PERCENT as u128,
        None => floor * UNMEASURED_PERMILLE as u128 * 100,
    };
    scaled.div_ceil(100_000) as Duffs
}

/// The ratios kept (BUG-319 r1 Should): between × 1 and × 10 of the floor. One outside is a bad
/// read (a fee below the formula's own floor, or a credits read that missed), not the network:
/// kept, it would raise the fee shown for the next ten creations.
pub const SANE_PERMILLE: std::ops::RangeInclusive<u32> = 1_000..=10_000;

/// `fee` against `floor`, in thousandths, rounded up; `None` without a floor to measure by.
pub fn ratio(fee: Duffs, floor: Duffs) -> Option<u32> {
    (floor > 0 && fee > 0).then(|| ((fee as u128 * 1000).div_ceil(floor as u128)) as u32)
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct Measured {
    /// Oldest first.
    ratios: Vec<u32>,
}

pub struct PoolFees {
    store: crate::json_file::Store<Measured>,
}

impl PoolFees {
    /// The file for `network` in the data folder (`pool-fees-testnet.json`, …).
    pub fn new(data_dir: &std::path::Path, network: &str) -> Self {
        Self::at(data_dir.join(format!("pool-fees-{network}.json")))
    }

    pub fn at(path: PathBuf) -> Self {
        Self {
            store: crate::json_file::Store::new(path),
        }
    }

    /// The ratios kept, oldest first; a missing or unreadable file is none.
    pub fn ratios(&self) -> Vec<u32> {
        self.store.read(|m| m.ratios.clone())
    }

    /// A creation's real fee against the floor shown then; the oldest goes past [`KEPT`]. One
    /// outside [`SANE_PERMILLE`] is not kept: `Err` says so, for the log.
    pub fn record(&self, fee: Duffs, floor: Duffs) -> Result<(), String> {
        let Some(r) = ratio(fee, floor) else {
            return Ok(());
        };
        if !SANE_PERMILLE.contains(&r) {
            return Err(format!(
                "a fee of {fee} against a floor of {floor} (× {r}/1000) is not kept"
            ));
        }
        self.store
            .update(|m| {
                m.ratios.push(r);
                let over = m.ratios.len().saturating_sub(KEPT);
                m.ratios.drain(..over);
                ((), true)
            })
            .map_err(|e| format!("{e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-319 (the architect's test): with 1.66 and 2.17 measured, the fee shown is at least
    /// floor × 2.17 × 1.15; with none, × 2.5 — not the old constant × 1.8 (the mutant).
    #[test]
    fn the_fee_shown_is_above_every_measurement() {
        let floor = 155_140;
        let shown = upper(floor, &[1_660, 2_170]);
        assert!(shown as f64 >= floor as f64 * 2.17 * 1.15, "{shown}");
        assert_eq!(shown, (155_140u64 * 2_170 * 115).div_ceil(100_000) as Duffs);
        assert_eq!(upper(floor, &[]), 387_850, "× 2.5 with nothing measured");
        assert!(upper(floor, &[]) > (floor * 18 + 9) / 10, "not × 1.8");
        assert!(upper(floor, &[1_660]) > 257_687, "above what T-1 took");
    }

    #[test]
    fn a_ratio_is_rounded_up_and_needs_a_floor() {
        assert_eq!(ratio(336_000, 155_140), Some(2_166));
        assert_eq!(ratio(1, 0), None);
        assert_eq!(ratio(0, 155_140), None);
    }

    /// r1 Should: a ratio outside × 1…× 10 is refused and said, the kept ones untouched.
    #[test]
    fn a_ratio_outside_one_to_ten_is_not_kept() {
        let dir = std::env::temp_dir().join(format!("evolite-pool-fees-x-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fees = PoolFees::new(&dir, "testnet");
        fees.record(336_000, 155_140).unwrap();
        assert!(fees.record(100_000, 155_140).is_err(), "below the floor");
        assert!(fees.record(1_600_000, 155_140).is_err(), "over × 10");
        assert!(
            fees.record(1_551_400, 155_140).is_ok(),
            "× 10 itself is kept"
        );
        assert_eq!(fees.ratios(), vec![2_166, 10_000]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The last ten are kept, oldest first; the file is per network.
    #[test]
    fn the_last_ten_are_kept_per_network() {
        let dir = std::env::temp_dir().join(format!("evolite-pool-fees-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let testnet = PoolFees::new(&dir, "testnet");
        assert!(testnet.ratios().is_empty(), "a missing file is none");
        for i in 0..12 {
            testnet.record(155_140 + i * 1_000, 155_140).unwrap();
        }
        let kept = PoolFees::new(&dir, "testnet").ratios();
        assert_eq!(kept.len(), KEPT);
        assert_eq!(
            kept[0],
            ratio(155_140 + 2_000, 155_140).unwrap(),
            "the two oldest gone"
        );
        assert!(
            PoolFees::new(&dir, "mainnet").ratios().is_empty(),
            "apart per network"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
