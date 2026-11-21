//! FEAT-0041: the BIP-39 seed with its passphrase — one way to it ([`crate::seed_from`]), the
//! published vectors, NFKD, and the guard that nothing builds keys from the phrase any other way.

use crate::{keys_from, seed_from};
use evolite_common::hex;

/// The BIP-39 test vector's phrase: a published test key, never a wallet's.
const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// The first BIP44 receiving address of the wallet a seed makes, on this network.
fn first_address(seed: &[u8; 64]) -> String {
    let keys = keys_from(seed).unwrap();
    let path: key_wallet::bip32::DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();
    let public = key_wallet::dashcore::PublicKey::new(keys.derive_public_key(&path).unwrap());
    key_wallet::dashcore::Address::p2pkh(&public, crate::network::dash()).to_string()
}

fn id(phrase: &str, passphrase: &str) -> String {
    hex(&crate::chain::wallet_id_of(&seed_from(phrase, passphrase).unwrap()).unwrap())
}

/// AC-A2, AC-T-P2: the official BIP-39 vector (Trezor's `vectors.json`, the first one: entropy
/// of zeros, passphrase «TREZOR»).
#[test]
fn the_bip39_vector_with_trezor() {
    assert_eq!(
        hex(&*seed_from(PHRASE, "TREZOR").unwrap()),
        "c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04"
    );
}

/// AC-A1: with no passphrase nothing changes — the wallet's id and its first address are those
/// key-wallet's `from_mnemonic` gave before FEAT-0041 (that is, `to_seed("")`).
#[test]
fn no_passphrase_is_the_wallet_as_before() {
    let before = key_wallet::Wallet::from_mnemonic(
        key_wallet::mnemonic::Mnemonic::from_phrase(PHRASE).unwrap(),
        crate::network::dash(),
        key_wallet::wallet::initialization::WalletAccountCreationOptions::Default,
    )
    .unwrap();
    let seed = seed_from(PHRASE, "").unwrap();
    let now = keys_from(&seed).unwrap();
    assert_eq!(now.compute_wallet_id(), before.compute_wallet_id());
    let path: key_wallet::bip32::DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();
    assert_eq!(
        now.derive_public_key(&path).unwrap(),
        before.derive_public_key(&path).unwrap()
    );
}

/// AC-A10, AC-T-P2: bip39 NFKDs the passphrase itself — «é» as one character and as two is one
/// seed, and so is a compatibility character and what it stands for (NFKD, not only NFD).
#[test]
fn the_passphrase_is_nfkd() {
    assert_eq!(
        *seed_from(PHRASE, "caf\u{e9}").unwrap(),
        *seed_from(PHRASE, "cafe\u{301}").unwrap()
    );
    assert_eq!(
        *seed_from(PHRASE, "\u{334d}").unwrap(),
        *seed_from(PHRASE, "\u{30e1}\u{30fc}\u{30c8}\u{30eb}").unwrap(),
        "㍍ is メートル"
    );
}

/// AC-A9, AC-A16, AC-T-P3: one phrase, two passphrases — two wallets: other ids and other
/// addresses; a space at the start is part of it (R-8).
#[test]
fn another_passphrase_is_another_wallet() {
    assert_ne!(id(PHRASE, ""), id(PHRASE, "TREZOR"));
    assert_ne!(id(PHRASE, "abc"), id(PHRASE, " abc"));
    assert_ne!(id(PHRASE, "abc"), id(PHRASE, "ABC"));
    assert_eq!(id(PHRASE, "TREZOR"), id(PHRASE, "TREZOR"));
    assert_ne!(
        first_address(&seed_from(PHRASE, "").unwrap()),
        first_address(&seed_from(PHRASE, "TREZOR").unwrap())
    );
}

/// AC-A3, AC-T-P4 (R-4): outside [`seed_from`] no source of the app builds a seed or keys from a
/// phrase — no `to_seed(`, `from_mnemonic(` or platform-wallet's `create_wallet_from_mnemonic`.
/// One that did would make keys of **another** wallet (the one with no passphrase). Tests are
/// not searched: they compare with the old way on purpose.
#[test]
fn no_seed_but_from_the_passphrase() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = Vec::new();
    for krate in [
        "evolite-ui",
        "evolite-core",
        "evolite-chain",
        "evolite-api",
        "evolite-local",
        "evolite-common",
        "evolite-demo",
    ] {
        let mut dirs = vec![crates.join(krate).join("src")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).expect("a source folder").flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if path.is_dir() {
                    if !matches!(name.as_str(), "tests" | "ui_tests") {
                        dirs.push(path);
                    }
                    continue;
                }
                let test_file = name == "tests.rs" || name.ends_with("_tests.rs");
                if !name.ends_with(".rs") || test_file {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("a source file");
                for (n, line) in code_lines(&text) {
                    if line.contains("to_seed(")
                        || line.contains("from_mnemonic(")
                        || line.contains("create_wallet_from_mnemonic")
                    {
                        found.push(format!("{}:{n}", path.display()));
                    }
                }
            }
        }
    }
    // The one: `seed_from` itself.
    assert_eq!(
        found.len(),
        1,
        "build keys from evolite_chain::seed_from: {found:#?}"
    );
    assert!(
        found[0]
            .replace('\\', "/")
            .contains("evolite-chain/src/lib.rs")
    );
}

/// The lines of `text` before its first test module, without comments, numbered from 1.
fn code_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        let test_attr = t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test");
        let next_is_mod = lines[i + 1..]
            .iter()
            .map(|l| l.trim_start())
            .find(|l| !l.starts_with("#["))
            .is_some_and(|l| {
                l.starts_with("mod ") || (l.starts_with("pub") && l.contains(" mod "))
            });
        if test_attr && next_is_mod {
            break;
        }
        if !t.starts_with("//") {
            out.push((i + 1, *line));
        }
    }
    out
}
