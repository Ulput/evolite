//! "Export CSV" (History): the wallet's history in the CSV that DashPay for Android writes
//! (Dash Wallet, `TaxBitExporter`): the TaxBit format, the same twelve columns and values, so
//! one tool reads the files of both wallets (the customer's decision, BUG-37).
//!
//! Rules taken from DashPay: one line per operation, oldest first; the type by the sign —
//! "Income" for what came in, "Expense" for what went out; what went out is all the wallet
//! lost, its fee included, and the "Fee" columns stay empty; time in UTC as
//! `2026-10-31T14:20:00Z`; amounts with a dot, two to eight decimals in pairs; the exchange id
//! column empty; `\n` between lines, no quotes (no field can hold a comma).
//!
//! Where evolite goes further (BUG-37): the Platform operations are lines too (credits, the
//! private pool, names), without a txid. For TaxBit the whole wallet is one account, so DASH
//! moved between its own places — its own Core addresses, Core to its identity or Platform
//! address, into the private pool, credits back to Core, credits to its own identity — is
//! not spent: only its fee is, one "Expense" line of the fee (as DashPay writes a transfer
//! to itself). The other half of such a move — the payout of a withdrawal, the note the
//! wallet made in the pool — is no line. Operations the network may still drop (being sent,
//! unconfirmed) are left out. An operation without a time of its own has an empty date and
//! goes last (BUG-47).

use evolite_api::{Activity, ActivityKind, Duffs, Proof, Timestamp, Wallet};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The header line, as DashPay writes it.
pub const HEADER: &str = "Date and Time,Transaction Type,Sent Quantity,Sent Currency,Sending Source,Received Quantity,Received Currency,Receiving Destination,Fee,Fee Currency,Exchange Transaction ID,Blockchain Transaction Hash";

/// The "source" and "destination" columns: DashPay names itself here, and so does evolite.
const SOURCE: &str = "evolite";

/// How the core names the payout of a credit withdrawal (`real/convert.rs`).
const WITHDRAWAL_PAYOUT: &str = "withdrawal from Platform";

/// What of the wallet is its own, to tell a move inside it from a payment to someone else.
#[derive(Default)]
pub struct Own {
    /// The names of its identities, lowercase, without `@`.
    names: HashSet<String>,
    /// Its Platform address and the Core addresses money came to.
    addresses: HashSet<String>,
}

impl Own {
    pub fn of(wallet: &Wallet) -> Self {
        let mut names: HashSet<String> = wallet
            .identities
            .iter()
            .filter_map(|i| i.name.as_deref())
            .map(str::to_lowercase)
            .collect();
        if let Some(identity) = &wallet.identity {
            names.extend(identity.names.iter().map(|n| n.label.to_lowercase()));
        }
        let mut addresses: HashSet<String> = wallet
            .activity
            .iter()
            .filter(|a| a.amount > 0)
            .filter_map(|a| a.address.clone())
            .collect();
        addresses.insert(wallet.core_address.clone());
        addresses.insert(wallet.platform_address.clone());
        addresses.remove("");
        Self { names, addresses }
    }

    /// Whether a payee (`@name` or an address) is this wallet.
    fn is(&self, to: &str) -> bool {
        match to.strip_prefix('@') {
            Some(name) => self.names.contains(&name.to_lowercase()),
            None => !to.is_empty() && self.addresses.contains(to),
        }
    }
}

/// What a line of the file says about an operation.
#[derive(Debug, PartialEq)]
enum Line {
    Income(Duffs),
    Expense(Duffs),
}

/// The line of `a`, if it is one.
fn line(a: &Activity, own: &Own) -> Option<Line> {
    // Not final: the network may still drop it.
    if matches!(a.proof, Proof::Sending | Proof::Unconfirmed) {
        return None;
    }
    let inside = match &a.kind {
        // The other half of a move inside the wallet: the payout of a withdrawal, the note a
        // shield made in the pool. Its fee is on the first half.
        ActivityKind::Received { from: Some(from) } if from == WITHDRAWAL_PAYOUT => return None,
        ActivityKind::Shielded { from: None } if a.amount >= 0 => return None,
        ActivityKind::SelfTransfer { .. }
        | ActivityKind::TopUp { .. }
        | ActivityKind::Shielded { .. } => true,
        ActivityKind::Withdrawal { to, .. } | ActivityKind::CreditTransfer { to } => own.is(to),
        ActivityKind::Sent { to } => own.is(to),
        _ => false,
    };
    if inside {
        // Only the fee is spent. One whose fee is not known: for a transfer to own addresses
        // its amount is the fee; otherwise nothing known was spent.
        let fee = match a.fee {
            Some(fee) if fee > 0 => fee,
            _ if matches!(a.kind, ActivityKind::SelfTransfer { .. }) => -a.amount,
            _ => 0,
        };
        return (fee > 0).then_some(Line::Expense(fee));
    }
    match a.amount {
        0 => None,
        n if n > 0 => Some(Line::Income(n)),
        n => Some(Line::Expense(-n)),
    }
}

/// The CSV of `activity` (any order), `own` telling moves inside the wallet apart.
pub fn taxbit(activity: &[Activity], own: &Own) -> String {
    let mut rows: Vec<(&Activity, Line)> = activity
        .iter()
        .filter_map(|a| Some((a, line(a, own)?)))
        .collect();
    // Oldest first; without a time, last.
    rows.sort_by_key(|(a, _)| (!crate::text::known(a.at), a.at));
    let mut out = String::from(HEADER);
    out.push('\n');
    for (a, l) in rows {
        let (kind, sent, received) = match l {
            Line::Income(n) => ("Income", String::new(), dash(n)),
            Line::Expense(n) => ("Expense", dash(n), String::new()),
        };
        let line = [
            if crate::text::known(a.at) {
                iso_utc(a.at)
            } else {
                String::new()
            },
            kind.into(),
            sent,
            "DASH".into(),
            SOURCE.into(),
            received,
            "DASH".into(),
            SOURCE.into(),
            // DashPay leaves the fee columns empty: the fee is in what was sent.
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
    use evolite_api::Chain;

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
    /// in the source columns (and a second past the epoch: 0 is "no time" here, BUG-47).
    #[test]
    fn writes_what_dashpay_writes() {
        let csv = taxbit(
            &[act(
                ActivityKind::Received { from: None },
                Chain::Core,
                1,
                1_000_000,
                "dd07f79a86185f3b5eeb2b19bdbc1d97eb9db0b887dcedb0fc30c622c43a27ba",
            )],
            &Own::default(),
        );
        let lines: Vec<&str> = csv.split('\n').collect();
        assert_eq!(lines[0], HEADER);
        assert_eq!(
            lines[1],
            "1970-01-01T00:00:01Z,Income,,DASH,evolite,0.01,DASH,evolite,,,,dd07f79a86185f3b5eeb2b19bdbc1d97eb9db0b887dcedb0fc30c622c43a27ba"
        );
        assert_eq!(lines[2], "");
    }

    fn with_fee(mut a: Activity, fee: Duffs) -> Activity {
        a.fee = Some(fee);
        a
    }

    /// The wallet of the table: its identity is @mine, `yOwn` one of its Core addresses.
    fn own() -> Own {
        Own {
            names: ["mine".to_owned()].into(),
            addresses: ["yOwn".to_owned()].into(),
        }
    }

    /// BUG-37, the analyst's table in DashPay's format (customer's decision analyst-33): the
    /// type by the sign, what went out with its fee in Sent Quantity, the Fee columns empty; a
    /// move inside the wallet is one Expense of its fee; the payout of a withdrawal, the
    /// wallet's own note in the pool and operations not final are no lines.
    #[test]
    fn every_operation_is_a_line_as_dashpay_writes_it() {
        use ActivityKind as K;
        let ops = vec![
            // From someone else.
            act(
                K::Received { from: None },
                Chain::Core,
                100,
                5_000_000,
                "aa",
            ),
            // To someone else: 0.001 and the fee.
            with_fee(
                act(
                    K::Sent {
                        to: "@ozzieq31".into(),
                    },
                    Chain::Core,
                    110,
                    -100_226,
                    "bb",
                ),
                226,
            ),
            // To own addresses, the fee known and not.
            with_fee(
                act(K::SelfTransfer { moved: 0 }, Chain::Core, 120, -226, "cc"),
                226,
            ),
            act(K::SelfTransfer { moved: 0 }, Chain::Core, 130, -300, "c2"),
            // A top-up of 0.01 of its own identity.
            with_fee(
                act(
                    K::TopUp {
                        identity: "@mine".into(),
                    },
                    Chain::CoreToPlatform,
                    140,
                    -1_000_263,
                    "dd",
                ),
                263,
            ),
            // Into the pool from Core (the asset lock), and its note in the pool.
            with_fee(
                act(
                    K::Shielded {
                        from: Some("Core".into()),
                    },
                    Chain::CoreToPlatform,
                    150,
                    -1_000_226,
                    "ee",
                ),
                226,
            ),
            act(
                K::Shielded { from: None },
                Chain::Platform,
                151,
                1_000_000,
                "",
            ),
            // Into the pool from the credits.
            with_fee(
                act(
                    K::Shielded {
                        from: Some("credits".into()),
                    },
                    Chain::Platform,
                    160,
                    -1_121_170,
                    "",
                ),
                121_170,
            ),
            // 0.01 of credits withdrawn to own Core, and its payout.
            with_fee(
                act(
                    K::Withdrawal {
                        to: "yOwn".into(),
                        status: None,
                    },
                    Chain::Platform,
                    170,
                    -1_215_053,
                    "",
                ),
                215_053,
            ),
            act(
                K::Received {
                    from: Some("withdrawal from Platform".into()),
                },
                Chain::Core,
                171,
                1_000_000,
                "ff",
            ),
            // Credits to someone else, and to its own identity.
            act(
                K::CreditTransfer { to: "@e".into() },
                Chain::Platform,
                180,
                -1_000,
                "",
            ),
            with_fee(
                act(
                    K::CreditTransfer { to: "@Mine".into() },
                    Chain::Platform,
                    190,
                    -1_100,
                    "",
                ),
                100,
            ),
            // A name.
            act(
                K::NameRegistered {
                    label: "kestrel".into(),
                },
                Chain::Platform,
                200,
                -57_979,
                "",
            ),
            // A note from someone else in the pool; a spend of nothing known.
            act(K::PoolReceived, Chain::Platform, 210, 70_000, ""),
            act(K::PoolSent, Chain::Platform, 220, 0, ""),
            // Not final yet.
            Activity {
                proof: Proof::Sending,
                ..act(K::Sent { to: "yX".into() }, Chain::Core, 230, -50_226, "gg")
            },
            Activity {
                proof: Proof::Unconfirmed,
                ..act(K::Sent { to: "yX".into() }, Chain::Core, 240, -50_226, "hh")
            },
        ];
        let csv = taxbit(&ops, &own());
        let lines: Vec<&str> = csv.lines().skip(1).collect();
        let t = |s: i64| iso_utc(s);
        let expense = |s, q: &str, tx: &str| {
            format!("{},Expense,{q},DASH,evolite,,DASH,evolite,,,,{tx}", t(s))
        };
        let expected = vec![
            format!("{},Income,,DASH,evolite,0.05,DASH,evolite,,,,aa", t(100)),
            expense(110, "0.00100226", "bb"),
            expense(120, "0.00000226", "cc"),
            expense(130, "0.000003", "c2"),
            expense(140, "0.00000263", "dd"),
            expense(150, "0.00000226", "ee"),
            expense(160, "0.00121170", ""),
            expense(170, "0.00215053", ""),
            expense(180, "0.000010", ""),
            expense(190, "0.000001", ""),
            expense(200, "0.00057979", ""),
            format!("{},Income,,DASH,evolite,0.0007,DASH,evolite,,,,", t(210)),
        ];
        assert_eq!(lines, expected, "{csv}");
        assert!(!csv.contains("Transfer"));
    }

    /// A payment to a contact that is an identity of this wallet moves nothing out of it
    /// (rule 3): only its fee.
    #[test]
    fn a_payment_to_its_own_identity_is_its_fee() {
        let paid = with_fee(
            act(
                ActivityKind::Sent { to: "@MINE".into() },
                Chain::Core,
                100,
                -100_226,
                "bb",
            ),
            226,
        );
        let csv = taxbit(&[paid], &own());
        assert_eq!(
            csv.lines().nth(1),
            Some("1970-01-01T00:01:40Z,Expense,0.00000226,DASH,evolite,,DASH,evolite,,,,bb")
        );
    }

    /// BUG-47: a note found in the pool has no time: an empty date, after everything dated —
    /// not 1970 at the top.
    #[test]
    fn an_operation_without_a_time_has_no_date_and_goes_last() {
        let csv = taxbit(
            &[
                act(
                    ActivityKind::PoolReceived,
                    Chain::Platform,
                    0,
                    1_000_000,
                    "",
                ),
                act(
                    ActivityKind::Received { from: None },
                    Chain::Core,
                    1_762_300_000,
                    5_000_000,
                    "aa",
                ),
            ],
            &Own::default(),
        );
        let lines: Vec<&str> = csv.lines().skip(1).collect();
        assert!(lines[0].ends_with(",aa"), "{csv}");
        assert_eq!(lines[1], ",Income,,DASH,evolite,0.01,DASH,evolite,,,,");
        assert!(!csv.contains("1970"));
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
    /// Every line must have the twelve columns and, where there is one, a txid of 64 hex digits.
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
        let csv = taxbit(&wallet.activity, &Own::of(&wallet));
        println!("{csv}");
        for line in csv.lines().skip(1) {
            let cols: Vec<&str> = line.split(',').collect();
            assert_eq!(cols.len(), 12, "{line}");
            // Platform operations have no Core transaction.
            let txid = cols[11];
            assert!(
                txid.is_empty()
                    || (txid.len() == 64 && txid.chars().all(|c| c.is_ascii_hexdigit()))
            );
            // DashPay's format: the type by the sign, the fee columns empty (BUG-37).
            assert!(cols[1] == "Income" || cols[1] == "Expense", "{line}");
            assert!(cols[8].is_empty() && cols[9].is_empty(), "{line}");
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
