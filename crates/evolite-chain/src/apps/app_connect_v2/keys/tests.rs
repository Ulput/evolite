//! The DIP's vectors ("Session Key Leaf Vectors", "Response Vector", "Key Derivation Vectors";
//! dip-pasta-app-connect.md, MIT License, Copyright (c) 2026 Dash Core Group, Inc.):
//! https://github.com/PastaPastaPasta/dips/blob/dec23c7f2a5efe94434f9c18c19e04e6e323960f/dip-pasta-app-connect.md

use super::*;

fn hex(text: &str) -> Vec<u8> {
    let text: String = text.split_whitespace().collect();
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

fn hex32(text: &str) -> [u8; 32] {
    hex(text).try_into().expect("32 bytes")
}

fn id(base58: &str) -> [u8; 32] {
    dpp::platform_value::string_encoding::decode(
        base58,
        dpp::platform_value::string_encoding::Encoding::Base58,
    )
    .expect("base58")
    .try_into()
    .expect("32 bytes")
}

fn app_ephemeral() -> [u8; 33] {
    hex("03e83cb927e85f96ae50a95ca3be21cc7a3755139a941b15d99b01af4fecf0d270")
        .try_into()
        .unwrap()
}

fn public_hex(secret: &SecretKey) -> String {
    PublicKey::from_secret_key(secret)
        .serialize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn the_request_id_of_each_binding_is_the_dips() {
    assert_eq!(
        request_id(&app_ephemeral(), 0),
        hex32("a0ad9d4647bd9296611258b8f47db92b06bdaee8782fd28e51632403d8a66a1e")
    );
    assert_eq!(
        request_id(&app_ephemeral(), 1),
        hex32("27a2c59bcf28894aee77c809c3c0e94201db08b66648c16fdfa689e6926b4f52")
    );
    // The bare hash is the earlier protocol's, not ours.
    assert_ne!(
        request_id(&app_ephemeral(), 0),
        hex32("18f020830ef82d97c975df563a075665dc2679003ce6f985a4448a0dc607234a")
    );
}

/// The DIP's key rows, from `abandon … about` with an empty passphrase.
#[test]
fn session_and_encryption_keys_derive_as_the_dip_pins_them() {
    let keys = crate::keys_of(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .expect("the public test phrase");
    let asc: [u8; 32] = std::array::from_fn(|i| i as u8);
    let desc: [u8; 32] = std::array::from_fn(|i| 0xff - i as u8);
    let (k35, k6b) = ([0x35; 32], [0x6b; 32]);
    let key = |path: DerivationPath| {
        let secret = derive(&keys, &path).expect("derived");
        public_hex(&SecretKey::from_secret_bytes(*secret).expect("a key"))
    };
    let test = key_wallet::Network::Testnet;
    let main = key_wallet::Network::Mainnet;
    assert_eq!(
        key(session_key_path(test, k35, k6b)),
        "022c8b2e806244482374b1caf8306146dc03aad3b99a5954efd5e70a0eddd37a5d"
    );
    assert_eq!(
        key(session_key_path(test, asc, desc)),
        "0278ad78c1a220924bf0e06e1111c61422efeace068f30efa825604b436970b230"
    );
    let rows = [
        (
            k35,
            k6b,
            EncPurpose::Encryption,
            0,
            "031a882c119e2b1f8e0349b444d877afa2599efeef49a26930fd25518cb6e9d57a",
        ),
        (
            k35,
            k6b,
            EncPurpose::Encryption,
            1,
            "03d0f6f0f48d968b9554d8e7fe467d7135bacf29d5c4994ca593bd10ebeed696f6",
        ),
        (
            asc,
            desc,
            EncPurpose::Encryption,
            0,
            "02dfed57ddaaea8c0758e8da44513520038eb8d6b0b92109953c14d6ab1087259b",
        ),
        (
            asc,
            desc,
            EncPurpose::Decryption,
            0,
            "020bcde800f41ad1f924477a9502f111445224cc9ab70114ae4e5099ba02bdd034",
        ),
        (
            asc,
            desc,
            EncPurpose::Decryption,
            1,
            "021b97869c21933673076a938337c28848c8bf8094bbe6883e3a4acbf33c639072",
        ),
    ];
    for (identity, contract, purpose, rotation, expected) in rows {
        assert_eq!(
            key(encryption_key_path(
                main, identity, contract, purpose, rotation
            )),
            expected,
            "{purpose:?} {rotation}"
        );
    }
}

fn wallet_ephemeral() -> SecretKey {
    SecretKey::from_secret_bytes(hex32(
        "d1772740bcf476137df337bbe42122e82da08e11b632d6c3ff0d7281576e4127",
    ))
    .unwrap()
}

fn owner() -> [u8; 32] {
    id("AsDve9GcVemmG7hRFkVMpX2UVQupcoFo7EAg33m8PcTA")
}

fn slots() -> Vec<Zeroizing<[u8; SLOT]>> {
    [
        "7df2f40f15e6079e514d7e5cb9cb1861b89986906ed820e158c3144272df9586",
        "3a992d9275d082af7540d1333a134ff0572c2bb79567437daa88dee55af1571f",
        "b37bb0ae8d6bbf01b38e843470570d5158d1939762a88673dd6cdf71bc144753",
    ]
    .iter()
    .map(|s| Zeroizing::new(hex32(s)))
    .collect()
}

#[test]
fn the_response_is_the_dips_byte_for_byte() {
    assert_eq!(
        public_hex(&wallet_ephemeral()),
        "02ee49ae562aa74ea28f95094da6800db9244b1addc1499439ffc7382cd7862d5a"
    );
    let app = PublicKey::from_slice(&app_ephemeral()).unwrap();
    assert_eq!(
        envelope_key(&wallet_ephemeral(), &app).to_vec(),
        hex("b9837f16975f02d7c06df91441bbf3dad51ca7d65016178f34084247dd1b703f")
    );
    assert_eq!(
        associated_data(&app_ephemeral(), &owner()),
        hex("646173683a6170702d636f6e6e6563743a7632d77dc0da9df83a47755a15c42c5d9db9d7a7fd629296d642338cb3bfceeb55
             4285462719e15f004d0579ce53ea77fb6d86cd9ab9")
    );
    let nonce: [u8; 12] = hex("22bb26ec9e0d316c4fb36be2").try_into().unwrap();
    let payload = seal(
        &slots(),
        &wallet_ephemeral(),
        &app_ephemeral(),
        &owner(),
        nonce,
    )
    .expect("sealed");
    assert_eq!(payload.len(), 124, "28 + 32 · 3");
    assert_eq!(
        payload,
        hex("22bb26ec9e0d316c4fb36be20e3f2a358cce3609d09094351ce5e78f08e5253b0f9258616374753411c97faecb1fbde336c0
             1d7f08380095a0a7ffe3fe207f80e92a7adb7bd8731dffe3b96a6b2c363aa1302c4e82d60f75419aa1f6f3ac5522453f2b76
             1495fb3b7816ea8bfe66a210e14fdad8b7f7c15ffb3ef39b")
    );
}

/// The app opens it with its ephemeral key, and only for the owner it was sealed for.
#[test]
fn the_app_opens_the_response_only_as_its_owners() {
    let app_secret = SecretKey::from_secret_bytes(hex32(
        "83adc3b5a48a1f3938969be4e3d674210d09aec8a2c2cb2b43b22972afca8682",
    ))
    .unwrap();
    assert_eq!(
        PublicKey::from_secret_key(&app_secret).serialize(),
        app_ephemeral()
    );
    let nonce = [9u8; 12];
    let payload = seal(
        &slots(),
        &wallet_ephemeral(),
        &app_ephemeral(),
        &owner(),
        nonce,
    )
    .unwrap();
    let wallet = PublicKey::from_secret_key(&wallet_ephemeral());
    let opened = open(&payload, &app_secret, &wallet, &app_ephemeral(), &owner()).expect("opened");
    let expected: Vec<[u8; SLOT]> = slots().iter().map(|s| **s).collect();
    assert_eq!(opened, expected);
    assert!(
        open(&payload, &app_secret, &wallet, &app_ephemeral(), &[1; 32]).is_none(),
        "another owner"
    );
    let mut changed = payload.clone();
    changed[20] ^= 1;
    assert!(open(&changed, &app_secret, &wallet, &app_ephemeral(), &owner()).is_none());
}
