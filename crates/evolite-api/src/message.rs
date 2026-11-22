//! FEAT-0044: what a check of a signed message says (dash-qt's format).

/// R-3: the check's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verified {
    /// «The signature is valid: the message was signed with this address's key».
    Valid,
    /// «The signature does not match this address» (another key, or another message).
    NotThisAddress,
    /// «This is not a signature».
    NotASignature,
}

/// R-12: why an address cannot be checked against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// Not an address at all.
    NotAnAddress,
    /// An address of the other network: its name.
    OtherNetwork(String),
    /// Not P2PKH (P2SH): such an address has no message signatures.
    NotP2pkh,
}
