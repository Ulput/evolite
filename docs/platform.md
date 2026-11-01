# Dash Platform in evolite: checking the stack (2026-10-29)

Can Platform (identities, DPNS names, credits, apps) be connected to evolite's Rust core without
changing the rust-dashcore pin `0dcdd82`, and does it answer on testnet?

## Result

**Yes, but for now only with a Platform branch that is not merged yet.** A DPNS query on testnet
passed with proof verification: `dash.dash` → identity `2ZqDG9z97YZrhA3UuuoLfk4b4XQdaMgTUowS7hTw6wGK`
(the name is taken), and a made-up name is not registered and is free.

| Platform revision | Its rust-dashcore | With our `0dcdd82` |
|---|---|---|
| release v5.0.1 | `d631aac`, another branch (+140 / −41 against ours) | no |
| `v5.1-dev` `57b5e5f` | `aca0202`, 7 commits behind ours | no: `dpp` calls the old BLS API (`sign`, `verify`, `secure_verify_aggregates`), while our pin moved to Base SDK `addedfa` |
| PR [dashpay/platform#5637](https://github.com/dashpay/platform/pull/5637), `a637fbd` (open) | `7329126e`, 1 commit ahead of ours | **yes**: builds and works |

The extra commit `7329126e` only changes how dash-spv stores the masternode list; the API is the
same. PR #5637 also keeps the low-S rule for ECDSA signatures: without it Platform 5.1 would
accept signatures that 5.0 rejects — a consensus split.

## How it is built

- `dash-sdk` (feature `dpns-contract`, without default features — they bring mocks and test
  vectors) + `dapi-grpc` with the `client` feature (only dash-sdk's default features enable it).
- `platform-wallet` + `dpp` with the `ed25519-dalek` feature: platform-wallet uses
  `dpp::ed25519_dalek` but does not enable the feature — in their workspace a neighbouring crate
  enables it.
- `[patch."https://github.com/dashpay/rust-dashcore"]` to our `0dcdd82` and `[patch.crates-io]`
  for rust-bitcoin `7ba35c7` (as in the Platform workspace). The graph has one copy of dashcore,
  dash-spv, key-wallet and key-wallet-manager — platform-wallet's types and our core's match.
- **Base SDK — two copies**: `33fcc59` (Platform) and `addedfa` (rust-dashcore). It does not break
  the build; BLS types may need converting where dpp and dashcore meet. It goes away once the
  revisions converge.

## What the machine needs

- **protoc 25 or newer** — `dapi-grpc` and `tenderdash-proto` generate code during the build and
  find protoc as prost does: the `PROTOC` environment variable, else `protoc` in `PATH`.
  `tenderdash-proto` refuses versions below 25; CI uses 32.0, as Platform's CI does.
- **Long paths.** The Platform repository has paths longer than 260 characters (rs-sdk's test
  vectors). On Windows cargo unpacks its git dependency only with
  `git config --global core.longpaths true` (cargo's libgit2 reads the key; the system
  `LongPathsEnabled` setting is not needed). The build directory must be short too (blake3
  silently fails in deeply nested folders).

## Trust: quorum keys

Platform proofs are verified with quorum BLS keys. In the probe the keys came from
`rs-sdk-trusted-context-provider` — an HTTPS service run by Dash, which means trusting a server.
For evolite the right way is its own `ContextProvider` on top of dash-spv:
`DashSpvClient::get_quorum_at_height` returns a quorum from our BLS-verified masternode list,
with its public key, and `ContextProvider::fetch_quorum_public_key` can hand it over
asynchronously. Platform has no such implementation ready — that is our work, a couple of dozen
lines. The provider does not have to keep contracts (DPNS, DashPay): the SDK then fetches them
itself, also with proofs.

## Plan

1. Revisions: **the customer decided to wait for #5637 to be merged** (2026-10-29). We do not move
   to the PR branch; after the merge — Platform `v5.1-dev`, pin `0dcdd82` or newer, as the
   customer decides.
2. Connect `dash-sdk` to `evolite-core`; quorum keys from our dash-spv.
3. Name checks and lookup by name (`is_dpns_name_available`, `resolve_dpns_name`) — the "Send"
   (`@name`) and "Create an identity" screens.
4. Identities: creation through an asset lock (InstantLock, otherwise ChainLock), DIP-13 keys from
   the phrase through platform-wallet; then name registration.
5. Credits, apps, the private pool — screen by screen.
