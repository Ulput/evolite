# The core on platform-wallet

evolite's core runs on `platform-wallet` and `platform-wallet-storage` from dashpay/platform —
the same crates Dash Evo Tool runs on. Testnet only. Done in the `platform-wallet` branch, merged
into `master` on 2026-10-30. With it `master` moved from rust-dashcore `0dcdd82` (Dark Gravity Wave
difficulty checks) to `aca0202` from this Platform revision; the check comes back with the move to
Platform v5.1-dev after dashpay/platform#5637 (rust-dashcore `7329126e`).

## Versions

| What | Revision | Why |
|---|---|---|
| dashpay/platform | `575d961` (v5.1-dev, 2026-10-26) | Dash Evo Tool pins it (`v1.0-dev`, `weekly.20261027`) |
| rust-dashcore | `aca0202` | this Platform revision pins it |

Compared with the old `master`, this loses the Dark Gravity Wave header difficulty check and six
more dash-spv commits (`aca0202` → `0dcdd82`). The Platform 5.0.1 fixes for a network with mixed
versions (dashpay/platform#5561, #5560) are missing — this matters for mainnet, not for testnet.

When dashpay/platform#5637 is merged, the branch moves to Platform `v5.1-dev`
(rust-dashcore `7329126e` = `0dcdd82` + 1 commit): the customer's pin, the 5.0.1 fixes and X11 in
pure Rust, all together.

## Preparing the machine

1. **Long paths in git** (Windows). Cargo takes Platform from GitHub at `575d961`, and its
   repository has paths longer than 260 characters (rs-sdk's test vectors): without this cargo
   cannot unpack it.

   ```
   git config --global core.longpaths true
   ```

2. **protoc 25 or newer** in `PATH` (`winget install Google.Protobuf`), or its full path in
   `PROTOC`. CI uses 32.0, as Platform's CI does: `protoc-32.0-win64.zip` from protocolbuffers
   on GitHub, SHA-256 `89c178e9f878cb6dd561f677688e1eb2e757cb9ac84ae30b64436bac19646996`.

3. clang is **not needed**: X11 in rust-dashcore `aca0202` is written in C, and its copy in
   `vendor/rs-x11-hash` builds with the MSVC compiler (see `vendor/rs-x11-hash/EVOLITE.md`).

## What was checked on testnet (2026-10-30)

With a separate probe on the same revision: an identity through an asset lock, a DPNS name, a
transfer into the private pool, an identity funded from the pool, DashPay (profile, request,
acceptance), a credit transfer to one's own Platform address. Pitfalls:

- the context provider must know the DashPay contract, otherwise the write goes through but the
  check of its proof fails (and a retry creates a duplicate);
- `platform-wallet-storage` needs the `shielded` feature, otherwise the private pool cannot be
  connected;
- GroveDB proof verification is deeply recursive: the core's threads need an 8 MB stack;
- a new identity does not show up in a search by phrase right away.

## What the branch already does

- The core (`crates/evolite-core/src/real/`, its chain in `crates/evolite-chain/`) runs on `platform-wallet`: `chain.rs` holds the
  wallet manager, its SPV client and the SQLite store (`wallets/testnet/platform-wallet.db`,
  public data only). In the manager the wallet is watch-only: the core derives signing keys from
  the phrase on unlock and forgets them on lock. Locking no longer stops the sync: the history
  keeps updating.
- `platform-wallet` builds and broadcasts Core payments (`finalize_transaction` +
  `broadcast_finalized_transaction`); our key signs them.
- Checked on testnet on 2026-10-30: restore (12 transactions, balance), lock and unlock, a payment
  to oneself (`35af26a8…81f18c`, fee 226 duff, the nodes relayed it back).

## Next

1. ~~Quorum keys from our SPV~~ — done: `evolite-chain/src/provider.rs` (`SpvContextProvider`, like
   `SpvProvider` in DET). Live test `live_spv_proved_read`: masternode lists from scratch in 78 s,
   proved DPNS reads passed. The plan was: quorum keys for verifying Platform proofs come from our
   SPV (`SpvRuntime::get_quorum_public_key`, verified quorums only), not from a trusted HTTP
   service; system contracts (DPNS, DashPay) go into the provider up front.
2. ~~Checking and looking up DPNS names~~ — done: the name rules (`dpns.rs`: 3–63 characters,
   normalisation o→0, i/l→1, contested names) are shared by the mock and the core; `CheckName`
   and `ResolveName` in the real core are proved reads through the SDK (`chain.rs`). Until the
   masternode lists are loaded, the core answers `NotSynced` right away (in grey on screen: "We
   check the name once the masternode lists load: without them there is nothing to check
   Platform's answer with."), and sends the real answer by itself once they are ready. The screens
   tell a typo (red) from the network's state (grey). A payment to `@name` is refused for now with
   an explanation: a name gives an identity, not a Core address — that is a DashPay contact
   (step 5).
3. ~~Identities: search by phrase on sign-in, creation through an asset lock, name
   registration~~ — done (`evolite-chain/src/identity.rs`, `chain.rs`):
   - After an unlock or a new wallet, the core searches for the phrase's identities in the
     background (`discover_from_master`, slots from 0 up to a gap) as soon as the masternode lists
     are loaded. platform-wallet puts what it finds into the store together with DPNS names. The
     screen shows the identity with the lowest slot and its credits.
   - "Create an identity": before the asset lock the core searches for identities once more (so
     as not to take a used slot), checks that the name is free, and locks the top-up plus the name
     fee. Then `register_identity_with_funding` (InstantSend, otherwise ChainLock), then
     `register_name_with_external_signer`. Six keys, as in DET: MASTER, HIGH, CRITICAL, TRANSFER,
     and ENCRYPTION and DECRYPTION for DashPay. Our `AssetLockSigner` signs the asset lock,
     Platform's `SimpleSigner` signs the Platform transitions; private keys live only in these
     signers for the duration of the operation.
   - Checked on testnet on 2026-10-30 (`live_identity_found_and_created`): after a restore the
     slot 0 identity was found (with a name, 5 keys, credits); a new identity with a name was
     created in 11 s (InstantSend), and the name resolves to the new identity.
   - The core does not register contested names yet (they need a masternode vote) and explains
     why.
   - Resuming after an interruption: platform-wallet keeps the asset lock in the store until
     Platform takes it; before locking the money the core writes the name it was for into
     `wallets/testnet/pending-names.json` (the slot number and the name, nothing secret). While
     the lock exists but its slot has no identity (or the identity exists but the name does not),
     "Create an identity" shows "Making the identity stopped" and a "Continue" button
     (`Command::ResumeIdentity`): `register_identity_with_funding(FromExistingAssetLock)` from the
     step where it stopped, then the name; no new money is locked, and no new identity is created
     while the old one is pending. Locks found in the chain by a wallet restore
     (`RecoveredFromChain`) do not count: platform-wallet does not know whether Platform took
     them and gives them slot 0; the search finds the restored phrase's identities. "Stopped" is
     shown only after the identity search in this session.
   - Checked on testnet on 2026-10-30 (`live_resume_identity_after_a_restart`, run as two
     processes — like two launches of the app): a 0.013 tDASH lock and a "crash", then after a
     restart the lock was found and the identity finished with its name — in 10 and 11 s over two
     runs.
   - Not done: a name for an existing identity ("Add a name"), the identity's creation date
     (Platform does not store it; the "Made" row is hidden).
4. ~~Credits and the Platform address~~ — done (`chain.rs`: `platform_address`, `send_credits`):
   - The Platform address (`tdash1…`) is the next unused address of the Platform payment
     account; the balance is the sum of credits on all of the wallet's addresses. platform-wallet's
     background sync (`platform_address_sync`, its own thread, every 30 s) updates address
     balances; the core starts it once the masternode lists are loaded and rebuilds the snapshot
     after every pass.
   - "Send" from "Credits" (signed by the first identity's TRANSFER key): to `@name` — a credit
     transfer to that identity; to `tdash1…` — credits to a Platform address; to a Core address —
     a withdrawal (masternodes pay it out after a few blocks). A mainnet address is rejected.
     After a transfer the core re-reads the identity's balance.
   - From Core to `@name` and to a Platform address — refused for now with an explanation (they
     need a DashPay contact and an asset lock to a Platform address). Sending from a Platform
     address is not done yet.
   - Checked on testnet on 2026-10-30 (`live_credits_move_to_an_address_and_a_name`): 0.0001 DASH
     of the first identity's credits went to its own Platform address and to the name of a second
     identity of the same phrase; after a sync the Platform address balance grew from 10,000 to
     20,000 duff. The withdrawal to Core was not checked live.
5. ~~DashPay~~ — done (Figma screens 19–23; `evolite-chain/src/dashpay.rs`, `real/avatars.rs`, `chain.rs`):
   - DashPay sync is platform-wallet's background `dashpay_sync` (requests both ways, profiles,
     payments, every 15 s); it starts together with the Platform address sync.
   - The DIP-15 key work (ECDH, account reference, contactInfo) is a `SeedCrypto` from the
     phrase's keys while the wallet is open; the first identity's keys sign requests and the
     profile.
   - Contacts in the snapshot: established ones, our requests without an answer, incoming ones
     (except hidden ones); the core asks for contacts' DPNS names once each. "Hide" is only in the
     store on this computer; nothing is written to Platform.
   - A Core payment to `@name` goes only to a contact: `send_payment` to the next address from the
     key the contact sent. Not a contact — refused with an explanation; the screen offers a
     contact request.
   - Profile: name ≤ 25, "About" ≤ 140, a picture over https — the core downloads it so that
     platform-wallet records its hash. Contacts' avatars are downloaded once per link (https,
     ≤ 1 MB, 15 s), accepted only if the hash from the profile matches, and drawn by a separate
     process (`Artist`), like NFT pictures. The download tells the picture's host the computer's
     address — as with any DashPay client.
   - Checked on testnet on 2026-10-30 (`live_dashpay_profile_and_request`): the first identity's
     profile was written with a picture, and the picture arrived and matched its hash; a request
     to a second identity of the same phrase was sent and shows as outgoing. Accepting a request
     and paying a contact were not checked live in this core: they need a second party (acceptance
     and a B↔C contact were checked earlier with a probe on the same revision).
6. ~~Jembe NFTs~~ — done, display only (`evolite-chain/src/nfts.rs`, `chain.rs`: `jembe_gifts`):
   - The `jembe-nft` contract on testnet is `8WdoxVsS2gh8enEhzSZPtGSxmbDs4SDadeLzqoRt3b3A`
     (`contracts/deployments.json` in Jembe, `testnet:nft`). A `gift` is a numbered copy that can
     be transferred and sold; a `giftModel` holds the title, description, picture (raw deflate;
     format 1 = SVG, 2 = GLB), `imageHash` — the SHA-256 of the uncompressed picture, and the
     edition size.
   - The core reads the first identity's gifts with proofs through the `byOwner` index
     (`$ownerId`, `$createdAt`), then the models by id — each once, they never change. In the
     background once the masternode lists are loaded, and again every minute while the wallet is
     open.
   - The picture is unpacked in the core only to check the hash (pure Rust, at most 1 MB); the
     picture's key is the hash of what actually arrived; on a mismatch the card is marked. The
     sandboxed renderer draws it, as in the demo.
   - Provenance: only the contract owner creates gifts, so a gift with another owner was
     "received". When — Platform does not store it (the contract does not require
     `$transferredAt`), nor from whom; we will not make it up.
   - The "Collection" screen in the real core shows only what exists: "Loading NFTs from Dash
     Platform…", "You have no NFTs yet", "NFTs belong to an identity", an error; "Give", "Sell" and
     "Get a gift" are removed (the demo keeps "Give").
   - Checked on testnet on 2026-10-31 (`live_jembe_gifts`): the test identity has 3 gifts — Coffin
     Dance, a neon djembe, and a 3D diamond ring (GLB); all pictures matched their `imageHash`.
   - Next: "Give" (a document transfer, revision +1 — see Jembe), selling (`$price`).
7. The private pool, apps — screen by screen.
8. Check recovery after an app restart (history from the store, without a rescan).
