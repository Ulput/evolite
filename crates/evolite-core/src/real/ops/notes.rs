//! Payment notes: opening them with the wallet, editing one, keeping the note of a payment being sent (FEAT-0003).

use crate::real::*;

impl Real {
    /// The wallet's payment notes, opened with it (FEAT-0003): the key from the seed, the file
    /// read once here, off the loop's thread (up to 6 MB to decrypt, AC-T9), and kept in memory
    /// until the lock.
    pub(in crate::real) async fn open_notes(&self, seed: &[u8], wallet_id: &str) {
        let path = self.paths.notes.clone();
        let seed = Zeroizing::new(seed.to_vec());
        let wallet_id = wallet_id.to_owned();
        let notes =
            tokio::task::spawn_blocking(move || notes::Notes::open(path, &seed, &wallet_id))
                .await
                .ok();
        *self.notes.lock().unwrap_or_else(|e| e.into_inner()) = notes;
        *self.first_seen.lock().unwrap_or_else(|e| e.into_inner()) = FirstSeen::default();
    }

    /// The keys of `history` that get the rate now as operations first seen in this unlock
    /// (R-4, AC-T6). Every line known until the wallet first knows all its coins counts as
    /// known at the unlock: what a sync finds (an old or restored wallet's history) gets none.
    /// Lines older than [`RECENT`] neither: the rate of now is not theirs.
    pub(in crate::real) fn first_seen_keys(
        &self,
        history: &[Activity],
        caught_up: bool,
    ) -> Vec<String> {
        let Some(rate) = self.fiat.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            return Vec::new();
        };
        let now = now();
        let mut seen = self.first_seen.lock().unwrap_or_else(|e| e.into_inner());
        let keys = history
            .iter()
            .map(|a| a.key.as_str())
            .filter(|k| !k.is_empty());
        if !seen.settled {
            seen.at_unlock.extend(keys.map(str::to_owned));
            seen.settled = caught_up;
            return Vec::new();
        }
        let recent = history
            .iter()
            .filter(|a| a.at != 0 && now - a.at <= RECENT)
            .map(|a| a.key.as_str());
        let notes = self.notes.lock().unwrap_or_else(|e| e.into_inner());
        let Some(notes) = notes.as_ref() else {
            return Vec::new();
        };
        let FirstSeen {
            at_unlock, seen, ..
        } = &mut *seen;
        evolite_api::fiat::first_seen_rates(
            recent,
            at_unlock,
            seen,
            |k| notes.rate(k).is_some(),
            Some(&rate),
            now,
        )
    }
}

/// How old a line may be and still get the rate of now as first seen (FEAT-0008).
const RECENT: Timestamp = 60 * 60;

/// The history lines this unlock has seen (FEAT-0008, R-4).
#[derive(Default)]
pub(in crate::real) struct FirstSeen {
    /// The keys known before the wallet first knew all its coins.
    at_unlock: std::collections::HashSet<String>,
    /// That moment has passed: keys not in [`Self::at_unlock`] are new.
    settled: bool,
    /// When each new key was first seen.
    seen: HashMap<String, Timestamp>,
}

impl Real {
    /// Keeps `rate` for operations first seen, off the loop's thread.
    pub(in crate::real) async fn rate_first_seen(&self, keys: Vec<String>) {
        let Some(rate) = self.fiat.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            return;
        };
        if keys.is_empty() {
            return;
        }
        let notes = self.notes.clone();
        let kept = tokio::task::spawn_blocking(move || {
            if let Some(notes) = notes.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                let _ = notes.set_rates(&keys, &rate, false);
            }
        });
        let _ = turn::timed("notes_rates", kept).await;
    }

    /// `Command::SetNote`: the note of an operation of the history, written off the loop's
    /// thread. The text never goes into an error or the journal.
    pub(in crate::real) async fn set_note(&self, key: String, text: &str) -> Result<(), CoreError> {
        let text = crate::note_text::kept(text)
            .map_err(|_| CoreError::NotAvailable(crate::note_text::TOO_LONG.into()))?;
        if !["core:", "platform:", "pool:"]
            .iter()
            .any(|kind| key.starts_with(kind))
        {
            return Err(CoreError::NotAvailable(
                tr!("There is no such operation in the history").into(),
            ));
        }
        let notes = self.notes.clone();
        tokio::task::spawn_blocking(move || {
            let mut notes = notes.lock().unwrap_or_else(|e| e.into_inner());
            let notes = notes
                .as_mut()
                .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
            notes.set(&key, text).map_err(note_error)
        })
        .await
        .map_err(|_| CoreError::Failed("notes".into()))?
    }
}

/// Why a note was not saved, for "Details": the kind of failure, never the note.
fn note_error(e: notes::NoteError) -> CoreError {
    match e {
        notes::NoteError::TooMany => CoreError::NotAvailable(crate::note_text::TOO_MANY.into()),
        notes::NoteError::NotRead => CoreError::NotAvailable(
            tr!(
                "The notes file was not read: notes can be changed once the wallet is opened again"
            )
            .into(),
        ),
        notes::NoteError::Write(kind) => CoreError::Failed(format!("notes.enc: {kind}")),
    }
}

/// A send's note as the core keeps it: cleaned, `None` when empty; too long refuses the send
/// before anything leaves the wallet (the form cuts it to what a note holds).
pub(in crate::real) fn send_note(note: Option<NoteText>) -> Result<Option<String>, CoreError> {
    match note {
        None => Ok(None),
        Some(note) => crate::note_text::kept(&note.0)
            .map_err(|_| CoreError::NotAvailable(crate::note_text::TOO_LONG.into())),
    }
}

/// What a payment keeps on this computer once the network took it: the person's note and the
/// rate of the time it was signed (FEAT-0008, АРХ-2). Neither is ever sent or logged.
#[derive(Default)]
pub(in crate::real) struct Kept {
    pub note: Option<String>,
    pub rate: Option<FiatRate>,
}

impl Kept {
    pub fn is_empty(&self) -> bool {
        self.note.is_none() && self.rate.is_none()
    }

    /// Into the notes under the operation's `key`; a failed write keeps what it can.
    pub fn into_notes(self, notes: &mut notes::Notes, key: &str) {
        if let Some(text) = self.note {
            let _ = notes.set(key, Some(text));
        }
        if let Some(rate) = &self.rate {
            let _ = notes.set_rates(&[key.to_owned()], rate, true);
        }
    }
}

impl Real {
    /// The rate a payment keeps (АРХ-2): the one «Summary» showed, else the core's own, if it
    /// is in the setting's currency and fresh; nothing with «Fiat rate» off (AC-T1).
    pub(in crate::real) fn payment_rate(&self, shown: Option<FiatRate>) -> Option<FiatRate> {
        let own = self.fiat.lock().unwrap_or_else(|e| e.into_inner()).clone();
        evolite_api::fiat::payment_rate(shown, own.as_ref(), self.fiat_currency.as_deref(), now())
    }
}

/// Keeps a send's note once the network took it (FEAT-0003, AC-T3), off the loop's thread and
/// before the send's answer goes out. A wallet locked meanwhile keeps nothing: its key is gone.
/// A failed write is in the journal by the file's name and the kind of error only.
pub(in crate::real) async fn keep_note(
    notes: Arc<std::sync::Mutex<Option<notes::Notes>>>,
    keep: impl FnOnce(&mut notes::Notes) + Send + 'static,
) {
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(notes) = notes.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            keep(notes);
        }
    })
    .await;
}
