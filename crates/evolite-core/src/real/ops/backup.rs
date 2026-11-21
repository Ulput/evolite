//! FEAT-0045: the backup file — saved from the open wallet's records, restored into them. The
//! file's key and fingerprint come from the seed at the unlock and go with the lock; nothing of
//! the file — its path, its name, its counts — goes to the journal (R-14).

use crate::real::*;
use evolite_api::backup::{BackupError, Restored, Section};
use evolite_local::backup as file;

impl Real {
    /// The backup key of the wallet just opened (with its notes, `open_notes`).
    pub(in crate::real) fn open_backup_key(&self, seed: &[u8]) {
        *self.backup_key.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Arc::new(file::BackupKey::of(seed)));
    }

    fn backup_key(&self) -> Result<Arc<file::BackupKey>, BackupError> {
        self.backup_key
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or(BackupError::Locked)
    }

    /// What the file holds now (AC-T-B2): the notes with their deletions' marks, the address
    /// book, contacts' names, hidden requests, signed-in apps, and the interface's settings as
    /// given. Nothing of the vault, of operations not finished, or of what the network keeps.
    fn backup_contents(&self, settings: Option<String>) -> Result<file::Contents, BackupError> {
        let notes = self
            .notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(notes::Notes::records)
            .ok_or(BackupError::Locked)?;
        // Tokens off (FEAT-0037): none chosen.
        let tokens = self
            .tokens
            .file
            .as_ref()
            .map(|f| f.chosen())
            .unwrap_or_default();
        Ok(file::Contents {
            notes,
            address_book: self.address_book.entries(),
            contact_names: self.contact_names.records(),
            hidden_requests: self.hidden.all(),
            connected_apps: self.apps.all(),
            settings: settings.and_then(|s| serde_json::from_str(&s).ok()),
            nft: None,
            platform_history: self.journal.finished_entries(),
            top_up_credits: self.journal.top_ups(),
            name_requests: self.name_requests.load(),
            pool_history: self.pool_history.records(),
            tokens: tokens.0,
            hidden_tokens: tokens.1,
            contact_addresses: self.contact_paid.store().read(Clone::clone),
        })
    }

    /// `Command::SaveBackup`: sealed and written off the loop's thread, beside `path` and then
    /// in its place (`json_file::write`). The file's name is the answer.
    pub(in crate::real) async fn save_backup(
        &self,
        path: PathBuf,
        settings: Option<String>,
    ) -> Result<String, BackupError> {
        let key = self.backup_key()?;
        let contents = self.backup_contents(settings)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        tokio::task::spawn_blocking(move || {
            let bytes = file::seal(&key, network(), &contents)
                .map_err(|why| BackupError::Io(why.into()))?;
            evolite_local::json_file::write(&path, &bytes)
                .map_err(|e| BackupError::Io(evolite_common::diag::io_error_kind(&e).into()))
        })
        .await
        .map_err(|_| BackupError::Io("task".into()))??;
        Ok(name)
    }

    /// `Command::RestoreBackup`: the file read and opened whole before anything is written
    /// (R-10); then each section merged into its own file, each written atomically. A section
    /// that fails stops the rest: what was merged stays, and a repeat duplicates nothing (R-9).
    pub(in crate::real) async fn restore_backup(
        &self,
        path: PathBuf,
    ) -> Result<Restored, BackupError> {
        let key = self.backup_key()?;
        let contents = tokio::task::spawn_blocking(move || {
            let bytes = file::read(&path).map_err(open_error)?;
            file::open(&key, network(), &bytes).map_err(open_error)
        })
        .await
        .map_err(|_| BackupError::Io("task".into()))??;
        let mut done = Restored {
            settings: contents.settings.as_ref().map(|v| v.to_string()),
            ..Restored::default()
        };
        let partial = |done: &Restored, failed| BackupError::Partial {
            done: done.clone(),
            failed,
        };
        // The notes off the loop's thread: up to 6 MB to encrypt and write (AC-T9 of FEAT-0003).
        let notes = self.notes.clone();
        let from = contents.notes;
        let merged = tokio::task::spawn_blocking(move || {
            let mut notes = notes.lock().unwrap_or_else(|e| e.into_inner());
            notes.as_mut().map(|n| n.merge(&from))
        })
        .await
        .ok()
        .flatten();
        match merged {
            Some(Ok(m)) => {
                done.notes = m.added;
                done.notes_newer = m.newer;
                done.notes_kept = m.kept;
            }
            Some(Err(_)) | None => return Err(partial(&done, Section::Notes)),
        }
        done.addresses = self
            .address_book
            .merge(&contents.address_book)
            .map_err(|_| partial(&done, Section::AddressBook))?;
        done.contact_names = self
            .contact_names
            .merge(&contents.contact_names)
            .map_err(|_| partial(&done, Section::ContactNames))?;
        done.hidden_requests = self
            .hidden
            .merge(&contents.hidden_requests)
            .map_err(|_| partial(&done, Section::HiddenRequests))?;
        done.apps = self
            .apps
            .merge(&contents.connected_apps)
            .map_err(|_| partial(&done, Section::ConnectedApps))?;
        // The journals: finished records only, one known here kept (the architect, 2026-11-21).
        let journals = (|| -> Option<usize> {
            let mut n = self.journal.merge_backup(&contents.platform_history).ok()?;
            n += self.journal.merge_top_ups(&contents.top_up_credits).ok()?;
            n += self
                .name_requests
                .merge_backup(&contents.name_requests)
                .ok()?;
            n += self
                .pool_history
                .merge_backup(&contents.pool_history)
                .ok()?;
            n += self
                .contact_paid
                .merge_backup(&contents.contact_addresses)
                .ok()?;
            Some(n)
        })();
        done.records = journals.ok_or_else(|| partial(&done, Section::Journals))?;
        if let Some(file) = self.tokens.file.as_ref() {
            done.tokens = file
                .merge_backup(&contents.tokens, &contents.hidden_tokens)
                .map_err(|_| partial(&done, Section::Tokens))?;
        }
        Ok(done)
    }
}

fn open_error(e: file::OpenError) -> BackupError {
    match e {
        file::OpenError::Damaged => BackupError::Damaged,
        file::OpenError::Newer => BackupError::Newer,
        file::OpenError::OtherNetwork(n) => BackupError::OtherNetwork(n),
        file::OpenError::OtherWallet => BackupError::OtherWallet,
        file::OpenError::Read(kind) => BackupError::Io(kind.into()),
    }
}
