//! FEAT-0037 AC-T3: on real mainnet headers, dash-spv's header check (rust-dashcore 0dcdd823)
//! takes the headers as Dash Core mined them and refuses one whose difficulty is set lower than
//! Dark Gravity Wave requires. A peer past the last checkpoint can no longer serve a cheap chain.
//! Offline: the headers are a file (mainnet 1999976 to 2000005, from rust-dashcore's own tests).

use dash_spv::error::ValidationError;
use dash_spv::types::HashedBlockHeader;
use dash_spv::validation::{BlockHeaderValidator, Validator};
use key_wallet::dashcore::block::Header;
use key_wallet::dashcore::{CompactTarget, Network};

/// The height of the first header in the file.
const FIRST: u32 = 1_999_976;
/// The headers Dark Gravity Wave reads before the one it checks.
const HISTORY: usize = 24;
/// The bits of a target half of all hashes meet (regtest's).
const EASY: u32 = 0x207f_ffff;

/// Each line is «height hash header-in-hex». The header must hash to the hash written next to it.
fn mainnet_headers() -> Vec<Header> {
    include_str!("testdata/mainnet_headers_1999976.txt")
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let [height, hash, hex] = line.split(' ').collect::<Vec<_>>()[..] else {
                panic!("height, hash and header: {line}");
            };
            assert_eq!(height.parse::<u32>().unwrap(), FIRST + i as u32);
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                .collect();
            let header: Header = key_wallet::dashcore::consensus::deserialize(&bytes).unwrap();
            assert_eq!(
                header.block_hash().to_string(),
                hash,
                "the file's header {height}"
            );
            header
        })
        .collect()
}

/// The validator for the headers after the first [`HISTORY`], and those headers.
fn checked(headers: &[Header]) -> (BlockHeaderValidator, Vec<HashedBlockHeader>) {
    let validator = BlockHeaderValidator::new(
        Network::Mainnet,
        headers[..HISTORY].to_vec(),
        FIRST + HISTORY as u32 - 1,
    );
    let batch = headers[HISTORY..]
        .iter()
        .map(HashedBlockHeader::from)
        .collect();
    (validator, batch)
}

#[test]
fn mainnet_headers_as_mined_are_taken() {
    let headers = mainnet_headers();
    assert_eq!(headers.len(), 30);
    let (validator, batch) = checked(&headers);
    assert!(validator.validate(&batch).is_ok());
}

#[test]
fn a_mainnet_header_made_easier_than_dgw_is_refused() {
    let mut headers = mainnet_headers();
    let last = headers.len() - 1;
    let mined = headers[last].bits;
    // A cheap header, as a peer past the last checkpoint could make: a target nearly anything
    // meets, and a nonce found for it in a few tries. Its proof of work holds, so only the
    // difficulty check can refuse it.
    headers[last].bits = CompactTarget::from_consensus(EASY);
    while !headers[last].target().is_met_by(headers[last].block_hash()) {
        headers[last].nonce = headers[last].nonce.wrapping_add(1);
    }
    let (validator, batch) = checked(&headers);
    match validator.validate(&batch) {
        Err(ValidationError::InvalidDifficulty {
            height,
            actual,
            required,
        }) => {
            assert_eq!(height, FIRST + last as u32);
            assert_eq!(actual, EASY);
            assert_eq!(
                required,
                mined.to_consensus(),
                "DGW asks for the bits mined"
            );
        }
        other => panic!("refused for its difficulty, got {other:?}"),
    }
}
