# Dash Platform in evolite: the stack

The pins are in the root `Cargo.toml`: dashpay/platform `575d961` (v5.1-dev of 2026-10-26, the
revision Dash Evo Tool runs) and the rust-dashcore it pins, `aca0202`. How the core uses them:
`docs/platform-wallet.md`.

## How it is built

- `dash-sdk` (features `dpns-contract`, `dashpay-contract` and `wallet`, without default features —
  they bring mocks and test vectors) + `dapi-grpc` with the `client` feature (only dash-sdk's
  default features enable it).
- `platform-wallet` + `dpp` with the `ed25519-dalek` feature: platform-wallet uses
  `dpp::ed25519_dalek` but does not enable the feature — in their workspace a neighbouring crate
  enables it.
- X11 from rust-dashcore `aca0202` is C: `[patch.crates-io]` takes `vendor/rs-x11-hash`, which
  builds with MSVC.

## What the machine needs

- **The Rust release of `rust-toolchain.toml`**; rustup picks it up.
- **protoc 25 or newer** — `dapi-grpc` and `tenderdash-proto` generate code during the build and
  find protoc as prost does: the `PROTOC` environment variable, else `protoc` in `PATH`.
  `tenderdash-proto` refuses versions below 25; CI uses 32.0, as Platform's CI does.
- **Long paths.** The Platform repository has paths longer than 260 characters (rs-sdk's test
  vectors). On Windows cargo unpacks its git dependency only with
  `git config --global core.longpaths true` (cargo's libgit2 reads the key; the system
  `LongPathsEnabled` setting is not needed). The build directory must be short too (blake3
  silently fails in deeply nested folders).

## Trust: quorum keys

Platform proofs are verified with quorum BLS keys. evolite takes them from its own dash-spv
(`crates/evolite-chain/src/spv/provider.rs`, `SpvContextProvider`): verified masternode lists, no
HTTPS service to trust. The system contracts (DPNS, DashPay, Withdrawals) come from `dpp`; other
contracts the SDK fetches itself, with proofs.

## Moving the pins

The pin moved from rust-dashcore `0dcdd82` (with the Dark Gravity Wave check of headers) to
`aca0202` when the core moved onto platform-wallet. dashpay/platform#5637, which was to bring it
back, was closed unmerged on 2026-11-03; its commits went into #5726, merged into `v5.1-dev`
(rust-dashcore `7329126e`). The move is task E1-1: `docs/architecture/answers-analyst.md`,
АРХ-E1-3.
