//! The wallet's own files that did not read (BUG-204, BUG-212): which were set aside, for the
//! screens' banners, and «Try again» / «Start over» on one of them.

use crate::real::*;
use evolite_api::{LocalFile, SetAside};
use evolite_local::json_file::{SetAsideFile, State};

impl Real {
    /// The store of `file`.
    fn local_store(&self, file: LocalFile) -> &dyn SetAsideFile {
        match file {
            LocalFile::Journal => self.journal.stores().0,
            LocalFile::TopUps => self.journal.stores().1,
            LocalFile::NameRequests => self.name_requests.store(),
            LocalFile::ConnectedApps => self.apps.store(),
            LocalFile::ContactAddresses => self.contact_paid.store(),
            LocalFile::AddressBook => self.address_book.store(),
            LocalFile::ContactResend => self.resend_debt.store(),
            LocalFile::NftSeries => self.nft_series.store(),
        }
    }

    /// The files set aside, for the snapshot.
    pub(in crate::real) fn set_aside_files(&self) -> Vec<SetAside> {
        ALL.iter()
            .filter_map(|&file| match self.local_store(file).state() {
                State::Readable => None,
                State::SetAside { path, .. } => Some(SetAside {
                    file,
                    path: path.display().to_string(),
                }),
            })
            .collect()
    }

    /// `Command::TryFileAgain` (`again`) or `Command::StartFileOver`. No password: neither
    /// spends nor shows a key (R-3). The journal says which file and what was done, never where
    /// it is or what it holds (R-4).
    pub(in crate::real) fn local_file(
        &self,
        file: LocalFile,
        again: bool,
    ) -> Result<(), CoreError> {
        let store = self.local_store(file);
        let (op, result) = if again {
            match store.try_again() {
                State::Readable => ("back", Ok(())),
                State::SetAside { .. } => (
                    "still set aside",
                    Err(CoreError::NotAvailable(
                        tr!("The file still does not read").into(),
                    )),
                ),
            }
        } else {
            store.start_anew();
            ("started over", Ok(()))
        };
        // Review r1, Should 1: the journal's lines kept in memory meanwhile go to the file now.
        if file == LocalFile::Journal && result.is_ok() {
            self.journal.keep_pending();
        }
        crate::diag::record(crate::diag::Record::Storage {
            file: name(file).into(),
            op,
            error: "",
        });
        result
    }
}

const ALL: [LocalFile; 8] = [
    LocalFile::Journal,
    LocalFile::TopUps,
    LocalFile::NameRequests,
    LocalFile::ConnectedApps,
    LocalFile::ContactAddresses,
    LocalFile::AddressBook,
    LocalFile::ContactResend,
    LocalFile::NftSeries,
];

/// The file's kind for the journal.
fn name(file: LocalFile) -> &'static str {
    match file {
        LocalFile::Journal => "platform-history.json",
        LocalFile::TopUps => "top-up-credits.json",
        LocalFile::NameRequests => "name-requests.json",
        LocalFile::ConnectedApps => "connected-apps.json",
        LocalFile::ContactAddresses => "contact-addresses.json",
        LocalFile::AddressBook => "address-book.json",
        LocalFile::ContactResend => "contact-resend.json",
        LocalFile::NftSeries => "nft-series.json",
    }
}
