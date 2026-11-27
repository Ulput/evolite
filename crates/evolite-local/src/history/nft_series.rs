//! «Workshop» series of this wallet (FEAT-0038 R-13): each write's state kept before it is sent
//! and after, so «Continue» reads again what was sent without an answer and repeats nothing
//! done; and the highest serial each model ever issued from here, so a deleted one is not
//! issued again (R-6). Public data only: ids, serials, prices, states.
//! `nft-series.json` in the wallet's folder.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What a series does (as `evolite_api::series::SeriesKind`, kept on disk).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Mint,
    Price { price: u64 },
    Reprice { from: u64, price: u64 },
    Gift { to: String },
    Unprice,
    Delete,
}

/// Where one write stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Waiting,
    /// Kept before it is sent: if evolite stops here, «Continue» reads it again first.
    Sending,
    Done,
    Refused,
    /// Sent and not seen (BUG-243): not repeated until reads show it did not land.
    Unclear,
}

/// One write: one instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub serial: u32,
    /// The instance's document (base58); for a mint, known once it is written.
    pub doc: Option<String>,
    pub state: State,
    /// Why it was refused, in a few words (for the History card).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// When its send ended without an answer (unix seconds): the quiet runs from here (BUG-243).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<u64>,
    /// When a read past the quiet first showed it did not land (unix seconds): a second, far
    /// enough after, lets it be sent again (BUG-243 (7)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_since: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Series {
    pub id: u64,
    pub identity: String,
    pub contract: String,
    pub model: String,
    pub kind: Kind,
    pub items: Vec<Item>,
    pub created_at: u64,
    /// The credits it took so far, in duffs.
    #[serde(default)]
    pub spent: u64,
    /// The plate's «×»: no longer shown, still in History.
    #[serde(default)]
    pub closed: bool,
    /// BUG-307: the model's name when the series started, for its History line (none in a
    /// series kept before it was).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// BUG-307: a run wrote something and the credits before or after it were not read — `spent`
    /// is not all it took: its History line says the cost is not known, never a sum too small.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cost_unknown: bool,
}

impl Series {
    /// Nothing left to send or read again.
    pub fn finished(&self) -> bool {
        self.items
            .iter()
            .all(|i| matches!(i.state, State::Done | State::Refused))
    }

    pub fn count(&self, state: State) -> usize {
        self.items.iter().filter(|i| i.state == state).count()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    #[serde(default)]
    pub series: Vec<Series>,
    /// The highest serial each model (base58) issued from this wallet, kept past its series.
    #[serde(default)]
    pub issued: BTreeMap<String, u32>,
    #[serde(default)]
    pub next_id: u64,
}

pub struct NftSeries {
    store: crate::json_file::Store<File>,
}

impl NftSeries {
    pub fn new(path: PathBuf) -> Self {
        Self {
            store: crate::json_file::Store::new(path),
        }
    }

    /// The file, for «Local files».
    pub fn store(&self) -> &crate::json_file::Store<File> {
        &self.store
    }

    pub fn all(&self) -> Vec<Series> {
        self.store.read(|f| f.series.clone())
    }

    pub fn get(&self, id: u64) -> Option<Series> {
        self.store
            .read(|f| f.series.iter().find(|s| s.id == id).cloned())
    }

    /// The highest serial `model` issued from here, if any.
    pub fn issued(&self, model: &str) -> Option<u32> {
        self.store.read(|f| f.issued.get(model).copied())
    }

    /// A new series, its writes all waiting; its id.
    pub fn start(&self, mut series: Series) -> Result<u64, String> {
        self.change(|f| {
            f.next_id += 1;
            series.id = f.next_id;
            let id = series.id;
            f.series.push(series);
            (id, true)
        })
    }

    /// Sets the state of `serial`'s write in series `id` (with its document and why, when
    /// given). A mint's serial counts as issued from `Sending` on: it may have landed.
    pub fn set(
        &self,
        id: u64,
        serial: u32,
        state: State,
        doc: Option<String>,
        why: Option<String>,
    ) -> Result<(), String> {
        self.edit(id, serial, |item| {
            item.state = state;
            if doc.is_some() {
                item.doc = doc;
            }
            item.why = why;
        })
    }

    /// Changes `serial`'s write in series `id` with `change`; a mint's serial counts as issued
    /// once it may have landed (anything but waiting or refused).
    pub fn edit(&self, id: u64, serial: u32, change: impl FnOnce(&mut Item)) -> Result<(), String> {
        self.change(|f| {
            let Some(s) = f.series.iter_mut().find(|s| s.id == id) else {
                return ((), false);
            };
            let model = s.model.clone();
            let mint = s.kind == Kind::Mint;
            let Some(item) = s.items.iter_mut().find(|i| i.serial == serial) else {
                return ((), false);
            };
            change(item);
            let state = item.state;
            if mint && state != State::Waiting && state != State::Refused {
                let top = f.issued.entry(model).or_insert(0);
                *top = (*top).max(serial);
            }
            ((), true)
        })
    }

    /// Adds `duffs` to what series `id` took.
    pub fn spent(&self, id: u64, duffs: u64) -> Result<(), String> {
        self.change(|f| match f.series.iter_mut().find(|s| s.id == id) {
            Some(s) => {
                s.spent = s.spent.saturating_add(duffs);
                ((), true)
            }
            None => ((), false),
        })
    }

    /// BUG-307: a run of series `id` wrote with its cost not read.
    pub fn cost_unknown(&self, id: u64) -> Result<(), String> {
        self.change(|f| match f.series.iter_mut().find(|s| s.id == id) {
            Some(s) => {
                s.cost_unknown = true;
                ((), true)
            }
            None => ((), false),
        })
    }

    /// The plate's «×».
    pub fn close(&self, id: u64) -> Result<(), String> {
        self.change(|f| match f.series.iter_mut().find(|s| s.id == id) {
            Some(s) => {
                s.closed = true;
                ((), true)
            }
            None => ((), false),
        })
    }

    /// BUG-337: when evolite opens the wallet, a series with nothing left to send or read again
    /// is closed — its plate was the last run's news; its line stays in History and «Recent».
    /// One stopped with writes left keeps its plate («Continue»). How many were closed.
    pub fn close_finished(&self) -> Result<usize, String> {
        self.change(|f| {
            let mut closed = 0;
            for s in f.series.iter_mut().filter(|s| !s.closed && s.finished()) {
                s.closed = true;
                closed += 1;
            }
            (closed, closed > 0)
        })
    }

    fn change<R>(&self, change: impl FnOnce(&mut File) -> (R, bool)) -> Result<R, String> {
        self.store.update(change).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evolite-nft-series-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("nft-series.json")
    }

    fn mint(serials: &[u32]) -> Series {
        Series {
            id: 0,
            identity: "me".into(),
            contract: "c".into(),
            model: "m".into(),
            kind: Kind::Mint,
            items: serials
                .iter()
                .map(|&serial| Item {
                    serial,
                    doc: None,
                    state: State::Waiting,
                    why: None,
                    sent_at: None,
                    no_since: None,
                })
                .collect(),
            created_at: 1,
            spent: 0,
            closed: false,
            model_name: None,
            cost_unknown: false,
        }
    }

    /// R-13: each write's state is on the disk before it is sent and after; a new store reads
    /// it back as it was (evolite stopped in between).
    #[test]
    fn a_write_s_state_survives_a_restart() {
        let path = temp("restart");
        let series = NftSeries::new(path.clone());
        let id = series.start(mint(&[1, 2, 3])).unwrap();
        series
            .set(id, 1, State::Done, Some("d1".into()), None)
            .unwrap();
        series.set(id, 2, State::Sending, None, None).unwrap();
        let again = NftSeries::new(path);
        let s = again.get(id).unwrap();
        let states: Vec<State> = s.items.iter().map(|i| i.state).collect();
        assert_eq!(states, [State::Done, State::Sending, State::Waiting]);
        assert_eq!(s.items[0].doc.as_deref(), Some("d1"));
        assert!(!s.finished());
    }

    /// R-6: a serial counts as issued once it may have landed, and stays after the series is
    /// closed; a refused one does not count.
    #[test]
    fn the_highest_issued_serial_is_kept_past_its_series() {
        let series = NftSeries::new(temp("issued"));
        let id = series.start(mint(&[1, 2, 3])).unwrap();
        series.set(id, 1, State::Done, None, None).unwrap();
        series.set(id, 2, State::Unclear, None, None).unwrap();
        series
            .set(id, 3, State::Refused, None, Some("credits".into()))
            .unwrap();
        assert_eq!(series.issued("m"), Some(2));
        series.close(id).unwrap();
        assert_eq!(series.issued("m"), Some(2));
        assert_eq!(series.issued("other"), None);
    }

    /// BUG-337: plates of the last run's series did not go after a restart and piled up (4, then
    /// 6, the model off the first screen). Opened again: the finished ones are closed, done or
    /// refused; one with writes left stays for «Continue»; the closed stay in the file.
    #[test]
    fn finished_series_are_closed_when_the_wallet_opens() {
        let path = temp("close-finished");
        let series = NftSeries::new(path.clone());
        let done = series.start(mint(&[1])).unwrap();
        series.set(done, 1, State::Done, None, None).unwrap();
        let refused = series.start(mint(&[2])).unwrap();
        series.set(refused, 2, State::Refused, None, None).unwrap();
        let left = series.start(mint(&[3, 4])).unwrap();
        series.set(left, 3, State::Done, None, None).unwrap();
        let again = NftSeries::new(path);
        assert_eq!(again.close_finished(), Ok(2));
        let closed: Vec<(u64, bool)> = again.all().iter().map(|s| (s.id, s.closed)).collect();
        assert_eq!(closed, [(done, true), (refused, true), (left, false)]);
        assert_eq!(again.close_finished(), Ok(0), "once");
    }
}
