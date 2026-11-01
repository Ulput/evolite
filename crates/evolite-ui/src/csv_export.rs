//! "Export CSV" (History): the wallet's Core transactions in the CSV that DashPay for
//! Android writes (Dash Wallet 11.9.1, `TaxBitExporter`): the TaxBit format, the same twelve
//! columns, values and rules, so one tool reads the files of both wallets.
//!
//! Rules taken from DashPay: one line per Core transaction, oldest first; transactions between
//! the wallet's own addresses are left out; the amount is what the wallet gained or lost (the
//! fee included when it paid one); "Income" for what came in, "Expense" for what went out
//! (DashPay lets the person mark a transaction "Transfer-in/out", evolite does not yet); time in
//! UTC as `2026-10-31T14:20:00Z`; amounts with a dot, two to eight decimals in pairs; the fee,
//! fee currency and exchange id columns empty; `\n` between lines, no quotes (no field can
//! hold a comma).

use evolite_api::{Activity, ActivityKind, Chain, Duffs, Timestamp};
use std::path::{Path, PathBuf};

/// The header line, as DashPay writes it.
pub const HEADER: &str = "Date and Time,Transaction Type,Sent Quantity,Sent Currency,Sending Source,Received Quantity,Received Currency,Receiving Destination,Fee,Fee Currency,Exchange Transaction ID,Blockchain Transaction Hash";

/// The "source" and "destination" columns: DashPay names itself here, and so does evolite.
const SOURCE: &str = "evolite";

/// The CSV of `activity` (any order; only Core transactions with a txid go in).
pub fn taxbit(activity: &[Activity]) -> String {
    let mut rows: Vec<&Activity> = activity
        .iter()
        .filter(|a| {
            matches!(a.chain, Chain::Core | Chain::CoreToPlatform)
                && a.txid.is_some()
                && a.amount != 0
                && !matches!(a.kind, ActivityKind::SelfTransfer)
        })
        .collect();
    rows.sort_by_key(|a| a.at);
    let mut out = String::from(HEADER);
    out.push('\n');
    for a in rows {
        let (kind, sent, received) = if a.amount > 0 {
            ("Income", String::new(), dash(a.amount))
        } else {
            ("Expense", dash(-a.amount), String::new())
        };
        let line = [
            iso_utc(a.at),
            kind.into(),
            sent,
            "DASH".into(),
            SOURCE.into(),
            received,
            "DASH".into(),
            SOURCE.into(),
            String::new(),
            String::new(),
            String::new(),
            a.txid.clone().unwrap_or_default(),
        ]
        .join(",");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Duffs as DashPay's `MonetaryFormat.BTC.noCode()` writes them: a dot, at least two
/// decimals, more in pairs only when needed (`0.01`, `0.0010`, `0.00012345`).
fn dash(duffs: Duffs) -> String {
    let whole = duffs / 100_000_000;
    let mut decimals = format!("{:08}", duffs % 100_000_000);
    while decimals.len() > 2 && decimals.ends_with("00") {
        decimals.truncate(decimals.len() - 2);
    }
    format!("{whole}.{decimals}")
}

fn iso_utc(at: Timestamp) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Where the file goes: the person's Downloads folder, else the home folder.
pub fn downloads() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let home = PathBuf::from(home);
    let downloads = home.join("Downloads");
    Some(if downloads.is_dir() { downloads } else { home })
}

/// Writes `csv` to `dir` as `transaction-history-<day>.csv` (DashPay's name, with the day
/// instead of a random part), `-2`, `-3`… when that name is taken. Answers the path.
pub fn save(dir: &Path, csv: &str, day: &str) -> std::io::Result<PathBuf> {
    for n in 1.. {
        let name = if n == 1 {
            format!("transaction-history-{day}.csv")
        } else {
            format!("transaction-history-{day}-{n}.csv")
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(csv.as_bytes())?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("a free name is found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::Proof;

    fn act(kind: ActivityKind, chain: Chain, at: Timestamp, amount: Duffs, txid: &str) -> Activity {
        Activity {
            id: 1,
            kind,
            chain,
            at,
            amount,
            proof: Proof::ChainLock,
            txid: (!txid.is_empty()).then(|| txid.to_owned()),
            fee: None,
            block: None,
            address: None,
        }
    }

    /// The header and line of DashPay's own test (`TransactionExportTest`), with evolite's name
    /// in the source columns.
    #[test]
    fn writes_what_dashpay_writes() {
        let csv = taxbit(&[act(
            ActivityKind::Received { from: None },
            Chain::Core,
            0,
            1_000_000,
            "dd07f79a86185f3b5eeb2b19bdbc1d97eb9db0b887dcedb0fc30c622c43a27ba",
        )]);
        let lines: Vec<&str> = csv.split('\n').collect();
        assert_eq!(lines[0], HEADER);
        assert_eq!(
            lines[1],
            "1970-01-01T00:00:00Z,Income,,DASH,evolite,0.01,DASH,evolite,,,,dd07f79a86185f3b5eeb2b19bdbc1d97eb9db0b887dcedb0fc30c622c43a27ba"
        );
        assert_eq!(lines[2], "");
    }

    #[test]
    fn keeps_core_transactions_oldest_first_and_leaves_out_the_rest() {
        let csv = taxbit(&[
            act(
                ActivityKind::Sent { to: "y1".into() },
                Chain::Core,
                200,
                -100_000_226,
                "bb",
            ),
            act(ActivityKind::SelfTransfer, Chain::Core, 150, -226, "cc"),
            act(
                ActivityKind::TopUp {
                    identity: "@d".into(),
                },
                Chain::CoreToPlatform,
                120,
                -100_263,
                "dd",
            ),
            act(
                ActivityKind::CreditTransfer { to: "@e".into() },
                Chain::Platform,
                110,
                -1_000,
                "",
            ),
            act(
                ActivityKind::Received { from: None },
                Chain::Core,
                100,
                5_000_000,
                "aa",
            ),
        ]);
        let lines: Vec<&str> = csv.lines().skip(1).collect();
        assert_eq!(lines.len(), 3, "{csv}");
        assert!(lines[0].starts_with("1970-01-01T00:01:40Z,Income,,DASH,evolite,0.05,"));
        assert!(lines[0].ends_with(",aa"));
        assert!(lines[1].starts_with("1970-01-01T00:02:00Z,Expense,0.00100263,DASH,evolite,,"));
        assert!(lines[2].contains(",Expense,1.00000226,") && lines[2].ends_with(",bb"));
    }

    #[test]
    fn amounts_as_dashpay_formats_them() {
        assert_eq!(dash(100_000_000), "1.00");
        assert_eq!(dash(1_000_000), "0.01");
        assert_eq!(dash(100_000), "0.0010");
        assert_eq!(dash(12_345), "0.00012345");
        assert_eq!(dash(12_345_670), "0.12345670");
        assert_eq!(dash(250_000_000_000), "2500.00");
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("evolite-csv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = save(&dir, "a\n", "2026-10-31").unwrap();
        let second = save(&dir, "b\n", "2026-10-31").unwrap();
        assert!(first.ends_with("transaction-history-2026-10-31.csv"));
        assert!(second.ends_with("transaction-history-2026-10-31-2.csv"));
        assert_eq!(std::fs::read_to_string(second).unwrap(), "b\n");
    }

    /// The real history of the test wallet: a copy of `%TEMP%\evolite-live-warm` (made by the
    /// core's live tests) is opened, synced, and its history written as the CSV, printed here.
    /// Every line must have the twelve columns and a txid of 64 hex digits.
    /// `cargo test -p evolite-ui live_csv -- --ignored --nocapture`
    #[cfg(feature = "real")]
    #[test]
    #[ignore = "needs testnet access and the warm wallet of the core's live tests"]
    fn live_csv_of_the_test_wallet() {
        use evolite_api::{Command, Event, Password, SyncStage};
        use std::time::{Duration, Instant};
        let warm = std::env::temp_dir().join("evolite-live-warm");
        assert!(warm.join("wallets").exists(), "run a core live test first");
        let dir = std::env::temp_dir().join("evolite-live-csv");
        let _ = std::fs::remove_dir_all(&dir);
        copy(&warm, &dir);
        let core = evolite_core::Core::real(dir, evolite_core::ArtOptions::default(), || {});
        core.send(Command::Unlock {
            password: Password::new("evolite live tests".into()),
        });
        let deadline = Instant::now() + Duration::from_secs(15 * 60);
        let wallet = loop {
            assert!(Instant::now() < deadline, "not synced");
            if let Some(w) = core.events().into_iter().find_map(|e| match e {
                Event::Snapshot(s) if s.sync.stage == SyncStage::Done => s.wallet,
                _ => None,
            }) {
                break w;
            }
            std::thread::sleep(Duration::from_millis(300));
        };
        let csv = taxbit(&wallet.activity);
        println!("{csv}");
        for line in csv.lines().skip(1) {
            let cols: Vec<&str> = line.split(',').collect();
            assert_eq!(cols.len(), 12, "{line}");
            assert!(cols[11].len() == 64 && cols[11].chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert!(
            csv.lines().count() > 1,
            "the test wallet has Core transactions"
        );
    }

    #[cfg(feature = "real")]
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}
