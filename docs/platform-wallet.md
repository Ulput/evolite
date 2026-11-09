# The core on platform-wallet

evolite's core runs on `platform-wallet` and `platform-wallet-storage` from dashpay/platform —
the same crates Dash Evo Tool runs on. Testnet by default; mainnet only with `--network mainnet`,
for checks with small amounts (open risk: `docs/architecture/notes.md`, R-6). The pins and what the
machine needs: `docs/platform.md`.

Compared with the earlier pin `0dcdd82`, `aca0202` lacks the Dark Gravity Wave header difficulty
check and six more dash-spv commits. The Platform 5.0.1 fixes for a network with mixed versions
(dashpay/platform#5561, #5560) are missing too — this matters for mainnet, not for testnet.

clang is **not needed** on Windows: X11 in rust-dashcore `aca0202` is written in C, and its copy in
`vendor/rs-x11-hash` builds with the MSVC compiler (see `vendor/rs-x11-hash/EVOLITE.md`).

## Where things are

| What | Where |
|---|---|
| The wallet manager, its SPV client, payments, identities, names, contacts | `crates/evolite-chain/src/spv/chain/` |
| Quorum keys from our SPV, system contracts (`SpvContextProvider`) | `crates/evolite-chain/src/spv/provider.rs` |
| Name rules, shared by the demo and the real core | `crates/evolite-common/src/dpns.rs` |
| Identity keys, DashPay crypto (`SeedCrypto`), Jembe NFTs, the pool | `crates/evolite-chain/src/platform/` |
| Operations and the core's state | `crates/evolite-core/src/real/` |
| The store | `wallets/<network>/<id>/platform-wallet.db`, public data only |

## Pitfalls

- the context provider must know the DashPay contract, otherwise the write goes through but the
  check of its proof fails (and a retry creates a duplicate);
- `platform-wallet-storage` needs the `shielded` feature, otherwise the private pool cannot be
  connected;
- GroveDB proof verification is deeply recursive: the core's threads need an 8 MB stack;
- a new identity does not show up in a search by phrase right away.

## How the core uses it

In the manager the wallet is watch-only: the core derives signing keys from the phrase on unlock
and forgets them on lock. Locking does not stop the sync. `platform-wallet` builds and broadcasts
Core payments (`finalize_transaction` + `broadcast_finalized_transaction`); our key signs them.

**Proved reads.** Until the masternode lists are loaded, `CheckName` and `ResolveName` answer
`NotSynced` at once, and the real answer follows by itself once they are ready.

**Identities.**
- After an unlock or a new wallet, the core searches for the phrase's identities in the background
  (`discover_from_master`, slots from 0 up to a gap) as soon as the masternode lists are loaded.
  By default the lowest slot is shown; the person can choose another, and every write "as the
  identity" follows that choice (`Chain::first_identity`).
- "Create an identity": before the asset lock the core searches once more (so as not to take a
  used slot), checks that the name is free, and locks the top-up plus the name fee. Then
  `register_identity_with_funding` (InstantSend, otherwise ChainLock), then the name. Six keys, as
  in DET: MASTER, HIGH, CRITICAL, TRANSFER, and ENCRYPTION and DECRYPTION for DashPay. Our
  `AssetLockSigner` signs the asset lock, Platform's `SimpleSigner` the transitions; private keys
  live only in these signers for the duration of the operation.
- Contested names are refused with an explanation (they need a masternode vote).
- Resuming: platform-wallet keeps the asset lock in the store until Platform takes it; before
  locking the money the core writes the name it was for into `pending-names.json` in the wallet's
  folder (the slot and the name, nothing secret). `Command::ResumeIdentity` continues
  (`FromExistingAssetLock`) from the step where it stopped; no new money is locked. Locks found by
  a wallet restore (`RecoveredFromChain`) do not count: platform-wallet does not know whether
  Platform took them.

**Credits and the Platform address.**
- The Platform address (`tdash1…`) is the next unused address of the Platform payment account; the
  balance is the sum of credits on all of the wallet's addresses. platform-wallet's background
  `platform_address_sync` (every 30 s) starts once the masternode lists are loaded.
- "Send" from "Credits" (the shown identity's TRANSFER key): to `@name` — a credit transfer; to
  `tdash1…` — credits to a Platform address; to a Core address — a withdrawal (masternodes pay it
  out after a few blocks).
- `Source` has only Core, Credits and Pool: a payment from a Platform address is not there. The
  Platform address can fund a shield (`ShieldSource::PlatformAddress`).

**DashPay.**
- Background `dashpay_sync` (requests both ways, profiles, payments, every 15 s) starts with the
  Platform address sync. The DIP-15 key work is a `SeedCrypto` from the phrase's keys while the
  wallet is open.
- "Hide" is only in the store on this computer; nothing is written to Platform.
- A Core payment to `@name` goes only to an established contact, to the address shown by the last
  `PreviewPayment`.
- Avatars are downloaded once per link (https, ≤ 1 MB, 15 s, public addresses only), accepted only
  if the hash from the profile matches, and drawn by the sandboxed renderer. The download tells the
  picture's host the computer's address — as with any DashPay client.

**Jembe NFTs** (display only; "Give" is refused by the real core).
- The `jembe-nft` contract on testnet is `8WdoxVsS2gh8enEhzSZPtGSxmbDs4SDadeLzqoRt3b3A`. A `gift`
  is a numbered copy; a `giftModel` holds the title, description, picture (raw deflate; format 1 =
  SVG, 2 = GLB), `imageHash` — the SHA-256 of the uncompressed picture, and the edition size.
- The core reads the shown identity's gifts with proofs, then the models by id. The picture is
  unpacked in the core only to check the hash (pure Rust, with a size limit); the sandboxed
  renderer draws it.

The private pool: `docs/architecture/shielded-pool.md`; apps:
`docs/architecture/apps-signing.md`.

## Checked live on testnet

Dates of the first pass; the tests are `#[ignore]` and named `live_*` (`docs/development.md`).

| When | What |
|---|---|
| 2026-10-30, a separate probe | an identity through an asset lock, a DPNS name, a transfer into the pool, an identity funded from the pool, DashPay (profile, request, acceptance), a credit transfer to one's own Platform address |
| 2026-10-30 | restore, lock and unlock, a payment to oneself |
| 2026-10-30, `live_spv_proved_read` | masternode lists from scratch in 78 s, proved DPNS reads |
| 2026-10-30, `live_identity_found_and_created` | the slot 0 identity found after a restore; a new identity with a name in 11 s |
| 2026-10-30, `live_resume_identity_after_a_restart` | a lock and a "crash", finished after a restart |
| 2026-10-30, `live_credits_move_to_an_address_and_a_name` | credits to a Platform address and to a name. The withdrawal to Core was not checked in this pass |
| 2026-10-30, `live_dashpay_profile_and_request` | a profile with a picture, a request. Acceptance and paying a contact need a second party |
| 2026-10-31, `live_jembe_gifts` | 3 gifts; all pictures matched their `imageHash` |
