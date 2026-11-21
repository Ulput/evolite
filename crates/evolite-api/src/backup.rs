//! FEAT-0045: the backup file — what the interface and the core say of it. The file itself and
//! its rules are `evolite_local::backup`'s.

/// What a backup file's name ends with.
pub const EXTENSION: &str = "evbackup";

/// The sections a restore merges, in the order it merges them (R-10: the order a «restored in
/// part» names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Notes,
    AddressBook,
    ContactNames,
    HiddenRequests,
    ConnectedApps,
    /// The journals: Platform's history, top-ups, name claims, the pool's, contacts' addresses.
    Journals,
    Tokens,
}

/// What a restore took from the file, by section: counts only (UC-2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Restored {
    /// Notes the wallet did not have.
    pub notes: usize,
    /// Notes the file had changed later than here.
    pub notes_newer: usize,
    /// Notes both had, kept as they are here.
    pub notes_kept: usize,
    pub addresses: usize,
    pub contact_names: usize,
    pub hidden_requests: usize,
    pub apps: usize,
    /// Lines of the journals the wallet did not have (history, top-ups, name claims, the pool's
    /// operations, contacts' paid addresses).
    pub records: usize,
    /// Tokens added by hand.
    pub tokens: usize,
    /// The wallet's settings the file holds, as the interface gave them when it was saved
    /// (JSON): the interface applies them. `None`: the file has none.
    pub settings: Option<String>,
}

/// Why a backup was not saved or not restored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupError {
    /// The wallet is locked: the file's key comes from its seed.
    Locked,
    /// «This file is from another wallet» (or the same phrase with another BIP-39 password).
    OtherWallet,
    /// «This file is for {}»: the network it was made on.
    OtherNetwork(String),
    /// «The file was made by a newer evolite. Update the app».
    Newer,
    /// «The file is damaged or changed».
    Damaged,
    /// The file could not be read or written: the io error's kind.
    Io(String),
    /// R-10: the sections before `failed` were merged (`done`), `failed` and the rest were not.
    /// A repeat duplicates nothing.
    Partial { done: Restored, failed: Section },
}
