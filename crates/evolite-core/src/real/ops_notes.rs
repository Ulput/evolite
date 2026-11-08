//! Payment notes: opening them with the wallet, editing one, keeping the note of a payment being sent (FEAT-0003).

use super::*;

impl Real {
    /// The wallet's payment notes, opened with it (FEAT-0003): the key from the seed, the file
    /// read once here, off the loop's thread (up to 6 MB to decrypt, AC-T9), and kept in memory
    /// until the lock.
    pub(super) async fn open_notes(&self, seed: &[u8], wallet_id: &str) {
        let path = self.paths.notes.clone();
        let seed = Zeroizing::new(seed.to_vec());
        let wallet_id = wallet_id.to_owned();
        let notes =
            tokio::task::spawn_blocking(move || notes::Notes::open(path, &seed, &wallet_id))
                .await
                .ok();
        *self.notes.lock().unwrap_or_else(|e| e.into_inner()) = notes;
    }

    /// `Command::SetNote`: the note of an operation of the history, written off the loop's
    /// thread. The text never goes into an error or the journal.
    pub(super) async fn set_note(&self, key: String, text: &str) -> Result<(), CoreError> {
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
pub(super) fn send_note(note: Option<NoteText>) -> Result<Option<String>, CoreError> {
    match note {
        None => Ok(None),
        Some(note) => crate::note_text::kept(&note.0)
            .map_err(|_| CoreError::NotAvailable(crate::note_text::TOO_LONG.into())),
    }
}

/// Keeps a send's note once the network took it (FEAT-0003, AC-T3), off the loop's thread and
/// before the send's answer goes out. A wallet locked meanwhile keeps nothing: its key is gone.
/// A failed write is in the journal by the file's name and the kind of error only.
pub(super) async fn keep_note(
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
