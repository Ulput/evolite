//! The example wallet from the Figma screens and the staged first sync it shows.

use super::answers::{PEOPLE, demo_contacts};
use crate::art::Stored;
use evolite_api::*;
use evolite_render::Packing;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The example phrase from Figma O1 (24 words; a 12-word wallet uses the first 12).
pub(super) const DEMO_PHRASE: [&str; 24] = [
    "orbit", "canvas", "lunar", "pepper", "alert", "frozen", "museum", "tackle", "ribbon",
    "velvet", "island", "sugar", "harbor", "violin", "meadow", "copper", "glide", "mercy",
    "theory", "basket", "flame", "olive", "tunnel", "quote",
];

pub(super) const TIP: u32 = 1_284_112;

pub(super) fn now() -> Timestamp {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as Timestamp)
}

/// The demo's wallets on this computer (Figma 26 and 27): three, the first open.
pub(super) fn demo_wallets(t: Timestamp) -> Vec<WalletEntry> {
    // "created 12 October 2026".
    const CREATED: Timestamp = 1_791_806_400;
    const DAY: Timestamp = 24 * 60 * 60;
    let entry = |id: &str, name: &str, color, origin, created_at, last_opened| WalletEntry {
        id: id.into(),
        name: name.into(),
        color,
        origin,
        created_at,
        last_opened,
        open: false,
        wrong_tries: 0,
        wait_until: None,
    };
    let mut wallets = vec![
        entry(
            "a1b2c3d4",
            tr!("Main"),
            0,
            WalletOrigin::Created,
            CREATED,
            Some(t),
        ),
        entry(
            "5e6f7a8b",
            tr!("Savings"),
            1,
            WalletOrigin::Restored,
            CREATED + 8 * DAY,
            Some(t - DAY),
        ),
        entry(
            "9c0d1e2f",
            tr!("Tests with Bob"),
            2,
            WalletOrigin::Created,
            CREATED + 15 * DAY,
            Some(CREATED + 15 * DAY),
        ),
    ];
    wallets[0].open = true;
    wallets
}

/// What the demo's data folder takes, as in Figma: "412 MB".
const DISK_BYTES: u64 = 412 * 1024 * 1024;

/// The example's Core nodes: eight, as the settings row says (documentation addresses).
fn demo_nodes() -> Vec<NodeInfo> {
    let roles = [
        NodeRole::Evonode,
        NodeRole::Evonode,
        NodeRole::Masternode,
        NodeRole::Masternode,
        NodeRole::Masternode,
        NodeRole::Regular,
        NodeRole::Regular,
        NodeRole::Regular,
    ];
    roles
        .into_iter()
        .enumerate()
        .map(|(i, role)| NodeInfo {
            address: format!("198.51.100.{}:19999", 11 + i * 7),
            role,
            banned_until: None,
        })
        .collect()
}

/// The example's DAPI nodes: five, one of them held off for a while.
fn demo_dapi() -> Vec<NodeInfo> {
    (0..5)
        .map(|i| NodeInfo {
            address: format!("https://203.0.113.{}:1443", 20 + i * 9),
            role: NodeRole::Evonode,
            banned_until: (i == 3).then_some(1_793_404_800),
        })
        .collect()
}

pub(super) fn synced() -> SyncInfo {
    SyncInfo {
        stage: SyncStage::Done,
        progress: Some(1.0),
        tip_height: TIP,
        headers_height: TIP,
        filters_height: TIP,
        core_peers: 8,
        platform_nodes: 5,
        minutes_left: None,
        chainlocked: true,
        error: None,
        disk_bytes: Some(DISK_BYTES),
        core_node: None,
        core_node_next: None,
        core_nodes: demo_nodes(),
        dapi_nodes: demo_dapi(),
    }
}

/// The pictures of the demo NFTs: three drawn for evolite and six of the samples made in
/// Jembe (three still, three animated with CSS) and its 3D ring, by file name.
const ART: [(&str, &[u8]); 10] = [
    ("jembe", include_bytes!("../../assets/nft/jembe.svg")),
    (
        "chainlocks",
        include_bytes!("../../assets/nft/chainlocks.svg"),
    ),
    ("genesis", include_bytes!("../../assets/nft/genesis.svg")),
    (
        "coffin-dance",
        include_bytes!("../../assets/nft/coffin-dance-animated.svg"),
    ),
    (
        "web-shot",
        include_bytes!("../../assets/nft/web-shot-animated.svg"),
    ),
    (
        "instantsend",
        include_bytes!("../../assets/nft/instantsend-neon.svg"),
    ),
    (
        "anon-hacker",
        include_bytes!("../../assets/nft/anon-hacker-flat.svg"),
    ),
    ("teddy", include_bytes!("../../assets/nft/teddy-pink.svg")),
    // From an issuer the wallet does not know: shown only after "Show".
    (
        "glitch-skull",
        include_bytes!("../../assets/nft/glitch-skull-neon.svg"),
    ),
    // A 3D model (glTF), as Jembe proposes to store them: `imageFormat` 2, GLB in raw deflate.
    (
        "diamond-ring",
        include_bytes!("../../assets/nft/diamond-ring-3d.glb"),
    ),
];

/// The key of a demo picture by its file name.
fn art(name: &str) -> String {
    let (_, bytes) = ART
        .iter()
        .find(|(n, _)| *n == name)
        .expect("a demo picture");
    crate::art::key(bytes)
}

/// The demo pictures by key, as the core keeps them. The Jembe samples are stored the way
/// Jembe's documents hold them (`imageFormat` 1: SVG in raw deflate, keyed by the SHA-256 of
/// the unpacked SVG); evolite's own three stay plain.
pub(super) fn demo_art() -> HashMap<String, Stored> {
    ART.iter()
        .enumerate()
        .map(|(i, (_, svg))| {
            let stored = if i < 3 {
                Stored {
                    bytes: Arc::from(*svg),
                    packing: Packing::Plain,
                }
            } else {
                Stored {
                    bytes: miniz_oxide::deflate::compress_to_vec(svg, 9).into(),
                    packing: Packing::RawDeflate,
                }
            };
            (crate::art::key(svg), stored)
        })
        .collect()
}

/// The example wallet from the Figma screens, with times relative to `t`.
pub(super) fn demo_wallet(t: Timestamp) -> Wallet {
    let min = 60;
    let hour = 60 * min;
    let day = 24 * hour;
    let act = |id, kind, chain, ago, amount, proof| Activity {
        id,
        kind,
        chain,
        at: t - ago,
        amount,
        proof,
        txid: None,
        fee: None,
        block: None,
        address: None,
    };
    use ActivityKind::*;
    Wallet {
        balances: Balances {
            core: 841_200_000,
            core_addresses: 3,
            credits: 120_000_000,
            // @satoshi-lab has no credits in the demo.
            all_credits: 120_000_000,
            platform_address: 73_400_000,
            shielded: 250_000_000,
        },
        activity: vec![
            act(
                1,
                PlatformReceived,
                Chain::Platform,
                40,
                12_000_000,
                Proof::Mempool,
            ),
            // Earlier receive addresses of ours: "Receive" lists them.
            Activity {
                address: Some("yPq8RkT2vW6nX4mB9cJ3hL7sD5fG1aZ8e4k".into()),
                ..act(
                    2,
                    Received {
                        from: Some("@alice".into()),
                    },
                    Chain::Core,
                    2 * hour,
                    25_000_000,
                    Proof::InstantLock,
                )
            },
            // What its card shows (`--screen activity-card`).
            Activity {
                txid: Some(
                    "536e90c01af419f42cd2b9430f318b26031c9532c86013a409d529b02fc802c4".into(),
                ),
                fee: Some(226),
                block: Some(1_284_101),
                address: Some("yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c".into()),
                ..act(
                    3,
                    Sent {
                        to: "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c".into(),
                    },
                    Chain::Core,
                    5 * hour,
                    -100_000_000,
                    Proof::ChainLock,
                )
            },
            act(
                4,
                TopUp {
                    identity: "@satoshi".into(),
                    creates: false,
                    credits: Credits::Added,
                },
                Chain::CoreToPlatform,
                day + 2 * hour,
                -50_000_000,
                Proof::ChainLock,
            ),
            act(
                5,
                AppSignIn {
                    app: "Yappr".into(),
                },
                Chain::Platform,
                day + 5 * hour,
                -20_000,
                Proof::Proved,
            ),
            act(
                6,
                Shielded { from: None },
                Chain::Platform,
                2 * day,
                -250_000_000,
                Proof::Proved,
            ),
            act(
                7,
                CreditTransfer { to: "@bob".into() },
                Chain::Platform,
                2 * day + hour,
                -5_000_000,
                Proof::Proved,
            ),
            Activity {
                address: Some("yNw3Hc6bV1qK8tR5mZ2xF9jD4sL7gP3aE6u".into()),
                ..act(
                    8,
                    Received {
                        from: Some(tr!("exchange").into()),
                    },
                    Chain::Core,
                    2 * day + 3 * hour,
                    500_000_000,
                    Proof::ChainLock,
                )
            },
        ],
        core_address: "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f2c".into(),
        platform_address: "tdash1qq7x4m9n2r8v5k3j6h0w4p2s8d5f7g9l3c0zq".into(),
        identity: Some(Identity {
            id: PEOPLE[0].id.into(),
            created_at: t - 17 * day,
            created_block: 1_271_004,
            revision: 3,
            key_source: tr!("evolite · DIP-13 keys, identity 0").into(),
            names: vec![
                DpnsName {
                    label: "satoshi".into(),
                    primary: true,
                    voting_days_left: None,
                },
                DpnsName {
                    label: "nakamoto-dev".into(),
                    primary: false,
                    voting_days_left: Some(9),
                },
            ],
            keys: vec![
                IdentityKey {
                    id: 0,
                    level: KeyLevel::Master,
                    purpose: KeyPurpose::Identity,
                    kind: KeyKind::Ecdsa,
                    decrypts: false,
                },
                IdentityKey {
                    id: 1,
                    level: KeyLevel::High,
                    purpose: KeyPurpose::Documents,
                    kind: KeyKind::Ecdsa,
                    decrypts: false,
                },
                IdentityKey {
                    id: 2,
                    level: KeyLevel::Critical,
                    purpose: KeyPurpose::Transfers,
                    kind: KeyKind::Ecdsa,
                    decrypts: false,
                },
                IdentityKey {
                    id: 3,
                    level: KeyLevel::Medium,
                    purpose: KeyPurpose::App("Yappr".into()),
                    kind: KeyKind::Ecdsa,
                    decrypts: false,
                },
                // Jembe's key on the Apps screen.
                IdentityKey {
                    id: 4,
                    level: KeyLevel::High,
                    purpose: KeyPurpose::App("Jembe".into()),
                    kind: KeyKind::Ecdsa,
                    decrypts: false,
                },
            ],
            profile: Some(PEOPLE[0].profile()),
        }),
        pool: Pool {
            notes: 3,
            checked_at: Some(now() - 4 * 60),
            address: Some(
                "tdash1zq8f5v6tnhx0ldqpw9c3ujmtk2yg7s4ea5r9lnvx3wd8kzp0q6myh2jhc7tq9ejw5aqr0"
                    .into(),
            ),
            unfinished_shield: None,
            connecting: false,
            operations: vec![
                act(
                    101,
                    Shielded {
                        from: Some("@satoshi".into()),
                    },
                    Chain::Platform,
                    2 * day - 3 * hour,
                    200_000_000,
                    Proof::Proved,
                ),
                act(
                    102,
                    PoolReceived,
                    Chain::Platform,
                    3 * day,
                    70_000_000,
                    Proof::Proved,
                ),
                act(
                    103,
                    PoolSent,
                    Chain::Platform,
                    4 * day,
                    -20_000_000,
                    Proof::Proved,
                ),
            ],
        },
        apps: vec![
            ConnectedApp {
                id: "yappr".into(),
                name: "Yappr".into(),
                domain: "yappr.app".into(),
                key: Some((3, KeyLevel::Medium, vec!["Yappr".into()])),
                last_used: t - 30 * min,
            },
            ConnectedApp {
                id: "jembe".into(),
                name: "Jembe".into(),
                domain: "jembe.app".into(),
                key: Some((
                    4,
                    KeyLevel::High,
                    vec!["jembe".into(), "jembe-private".into()],
                )),
                last_used: t - day,
            },
            ConnectedApp {
                id: "dashnotes".into(),
                name: "DashNotes".into(),
                domain: "notes.example".into(),
                key: None,
                last_used: t - 3 * day,
            },
        ],
        contacts: demo_contacts(t),
        unfinished_identity: None,
        unfinished_top_up: None,
        // Off in the demo, as the setting was before the core kept it: its screens stay one
        // click (Settings → Security turns it on).
        password_each_send: false,
        identity_search: IdentitySearch::Done,
        // Two identities, so the Identity screen shows its choice.
        address_book: vec![
            SavedAddress {
                address: "yNk2Hc8pWq4mR7vT3sLb9XeJ6uZa5Df1Gh".into(),
                label: tr!("Workshop rent").into(),
            },
            SavedAddress {
                address: "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5a9f2c".into(),
                label: tr!("Exchange · withdrawal").into(),
            },
        ],
        identities: vec![
            IdentityChoice {
                id: PEOPLE[0].id.into(),
                name: Some("satoshi".into()),
                slot: 0,
            },
            IdentityChoice {
                id: "5Kx2LabQ7pWmR9sTzV4nB8cYd1Je3HuF6gNqA2oX7wE".into(),
                name: Some("satoshi-lab".into()),
                slot: 1,
            },
        ],
        nfts: vec![
            Nft {
                id: "nft-jembe-12".into(),
                name: tr!("Djembe").into(),
                description: tr!("The beat every jam starts with. A gift for evolite's launch.")
                    .into(),
                art: art("jembe"),
                art_verified: true,
                origin: NftOrigin::Gift {
                    from: "@alice".into(),
                    at: t - 2 * day,
                },
                issuer: "jembe".into(),
                contract: "jembe-nft".into(),
                edition: (12, 500),
                owner: "@satoshi".into(),
                price: None,
            },
            Nft {
                id: "nft-chainlocks-3".into(),
                name: "ChainLocks".into(),
                description: tr!(
                    "A lock no miner can open: a block with a ChainLock cannot be rewritten."
                )
                .into(),
                art: art("chainlocks"),
                art_verified: true,
                origin: NftOrigin::Gift {
                    from: "@bob".into(),
                    at: t - 9 * day,
                },
                issuer: "jembe".into(),
                contract: "jembe-nft".into(),
                edition: (3, 100),
                owner: "@satoshi".into(),
                price: Some(50_000_000),
            },
            Nft {
                id: "nft-genesis-1".into(),
                name: tr!("Genesis block").into(),
                description: tr!(
                    "The first issue of the jembe collection: a sprout on the very first block."
                )
                .into(),
                art: art("genesis"),
                art_verified: true,
                origin: NftOrigin::Release,
                issuer: "jembe".into(),
                contract: "jembe-nft".into(),
                edition: (1, 21),
                owner: "@satoshi".into(),
                price: None,
            },
            Nft {
                id: "nft-coffin-dance-7".into(),
                name: "Coffin Dance".into(),
                description: tr!("The very pallbearers: they dance until the price comes back.")
                    .into(),
                art: art("coffin-dance"),
                art_verified: true,
                origin: NftOrigin::Gift {
                    from: "@alice".into(),
                    at: t - 3 * hour,
                },
                issuer: "jembe".into(),
                contract: "jembe-nft".into(),
                edition: (7, 300),
                owner: "@satoshi".into(),
                price: None,
            },
            Nft {
                id: "nft-web-shot-2".into(),
                name: "Web Shot".into(),
                description: tr!("A web in flight: a gift from the superhero jam.").into(),
                art: art("web-shot"),
                art_verified: true,
                origin: NftOrigin::Gift {
                    from: "@bob".into(),
                    at: t - 5 * day,
                },
                issuer: "jembe".into(),
                contract: "jembe-nft".into(),
                edition: (2, 300),
                owner: "@satoshi".into(),
                price: None,
            },
            Nft {
                id: "nft-instantsend-40".into(),
                name: "InstantSend".into(),
                description: tr!("A payment confirmed in a second: Dash's neon lightning.").into(),
                art: art("instantsend"),
                art_verified: true,
                origin: NftOrigin::Release,
                issuer: "jembe".into(),
                contract: "jembe-nft".into(),
                edition: (40, 1000),
                owner: "@satoshi".into(),
                price: Some(10_000_000),
            },
            Nft {
                id: "nft-anon-hacker-5".into(),
                name: tr!("Anonymous hacker").into(),
                description: tr!(
                    "A hood, a mask and blinking eyes: a cypherpunk from the jembe collection."
                )
                .into(),
                art: art("anon-hacker"),
                art_verified: true,
                origin: NftOrigin::Gift {
                    from: "@alice".into(),
                    at: t - 12 * day,
                },
                issuer: "jembe".into(),
                contract: "jembe-nft".into(),
                edition: (5, 50),
                owner: "@satoshi".into(),
                price: None,
            },
            Nft {
                id: "nft-diamond-ring-3".into(),
                name: tr!("Diamond ring").into(),
                description: tr!("A 3D gift: the ring turns slowly, the stone catches the light.")
                    .into(),
                art: art("diamond-ring"),
                art_verified: true,
                origin: NftOrigin::Gift {
                    from: "@alice".into(),
                    at: t - 30 * min,
                },
                issuer: "jembe".into(),
                contract: "jembe-nft".into(),
                edition: (3, 100),
                owner: "@satoshi".into(),
                price: None,
            },
            Nft {
                id: "nft-teddy-19".into(),
                name: "Teddy".into(),
                description: tr!("A pink teddy bear from the community collection.").into(),
                art: art("teddy"),
                art_verified: true,
                origin: NftOrigin::Release,
                issuer: "jembe".into(),
                contract: "jembe-nft".into(),
                edition: (19, 200),
                owner: "@satoshi".into(),
                price: None,
            },
            Nft {
                id: "nft-glitch-skull-1".into(),
                name: "Glitch Skull".into(),
                description: tr!(
                    "It came from an unknown address. The picture is drawn only if you ask."
                )
                .into(),
                art: art("glitch-skull"),
                art_verified: true,
                origin: NftOrigin::Gift {
                    from: "".into(),
                    at: t - 6 * hour,
                },
                issuer: "neon-forge".into(),
                contract: "neon-forge-drops".into(),
                edition: (1, 10_000),
                owner: "".into(),
                price: None,
            },
        ],
        nfts_status: NftsStatus::Ready,
        fees: Fees {
            core_payment: 522,
            // The Core balance in three coins (BUG-55): a payment costs the coins it takes.
            core_coins: vec![
                CoreCoin {
                    value: 500_000_000,
                    fee: 226,
                },
                CoreCoin {
                    value: 300_000_000,
                    fee: 374,
                },
                CoreCoin {
                    value: 41_200_000,
                    fee: 522,
                },
            ],
            credit_transfer: 2_000,
            pool_transfer: 114_140,
            nft_transfer: 30_000,
            document_write: 20_000,
            shield: 149_760,
            identity_create: 2_000,
            identity_lock_min: 241_000,
            name_regular: 1_000_000,
            name_contested: 20_000_000,
            add_key: 10_000,
            contact_request: 10_000,
            profile: 15_000,
            // Platform's pool formulas at protocol 15, two Orchard actions.
            shield_from_address: 114_140,
            shield_reserve: 228_280,
            unshield: 120_223,
            pool_withdrawal: 226_480,
            shield_from_core: 164_140,
            pool_top_up: 114_360,
            credit_withdrawal: 400_000,
            credit_withdrawal_reserve: 400_000,
        },
        phrase_words: 12,
        created_at: t - 17 * day,
    }
}

/// The staged first sync: filters from 68 % to 100 % over 30 s, then four seconds per stage.
pub(super) fn staged_sync(elapsed: f32) -> SyncInfo {
    let filters = (0.68 + elapsed / 30.0 * 0.32).min(1.0);
    let after = ((elapsed - 30.0) / 4.0).max(0.0) as usize;
    let stage = if filters < 1.0 {
        SyncStage::Filters
    } else {
        [
            SyncStage::Transactions,
            SyncStage::Platform,
            SyncStage::ChainLock,
            SyncStage::Done,
        ][after.min(3)]
    };
    SyncInfo {
        stage,
        // As the real core: the steps after the filters have nothing to count.
        progress: match stage {
            SyncStage::Filters => Some(filters),
            SyncStage::Done => Some(1.0),
            _ => None,
        },
        tip_height: TIP,
        headers_height: TIP,
        filters_height: (TIP as f32 * filters) as u32,
        core_peers: 8,
        platform_nodes: 5,
        minutes_left: (stage != SyncStage::Done).then(|| ((1.0 - filters) * 10.0).ceil() as u32),
        chainlocked: stage == SyncStage::Done,
        error: None,
        disk_bytes: Some(DISK_BYTES),
        core_node: None,
        core_node_next: None,
        core_nodes: demo_nodes(),
        dapi_nodes: demo_dapi(),
    }
}
