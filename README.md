<p align="center">
  <img src="docs/assets/evolite-logo.svg" alt="evolite" width="360">
</p>

# evolite

A light, **non-custodial** Dash wallet for Core and Platform, written in Rust with **egui**. The
recovery phrase and the keys exist only on your computer, encrypted with your password: no server
holds them, and nobody but you can move the funds. Everything the network sends is verified on
this computer: block headers and filters, ChainLocks and InstantLocks (BLS against its own
masternode lists), and Platform answers against GroveDB proofs.

**A testnet demo**: the coins are worth nothing. Mainnet is enabled only by the explicit flag
`--network mainnet` and is meant for checks with small amounts.

## What it does

- **Core** (dash-spv): receiving, sending (InstantSend, otherwise ChainLock), history with fees
  and CSV export, past addresses, an address book, payments in the mempool, your own node instead
  of DNS seeds.
- **Platform**: identities from the phrase (DIP-13) — discovery, creation through an asset lock
  that resumes after an interruption, several identities to choose from; DPNS names; credits, a
  Platform address, top-ups, withdrawals and transfers.
- **DashPay**: contacts and contact requests, a profile with an avatar, payments to addresses
  derived from a contact's key, importing a DashPay wallet on restore.
- **Private pool** (Orchard): shield into the pool, transfer inside it, unshield to Core or to an
  identity.
- **Apps**: sign-in and signing by link or QR code (`dash-key:`, `dash-st:`), a list of connected
  apps that can be disconnected. Sign-in by code (`dash-kx:`) is not there yet.
- **NFTs** (Jembe): a collection whose pictures are checked against their hash; pictures are drawn
  by a separate process with no access to the keys.
- **Several wallets**: each has its own phrase, password, history and settings; switching takes
  the password and no restart.
- **Security**: the phrase is encrypted with the password (Argon2id + AES-256-GCM), auto-lock, one
  running copy of the app per data folder, a phrase on the clipboard stays out of the Windows
  clipboard history and is cleared after 60 seconds.
- **Interface**: dark and light themes, 15 languages (Arabic right to left), the rate in a fiat
  currency (optional), system notifications on Windows, "Report a problem" by email with a journal
  free of personal data.

## Download

Installers for Windows, Linux and macOS (x86_64 and ARM64) are on the
[Releases](https://github.com/Ulput/evolite/releases) page, with their SHA-256 sums. They are not
code-signed: Windows SmartScreen warns about an unknown publisher, and on macOS open the
installer with right-click → Open.

## Building

Rust through rustup: `rust-toolchain.toml` pins the release, and rustup installs it (MSVC on
Windows). And protoc 25 or newer in `PATH`: `dapi-grpc` and `tenderdash-proto` generate code with
it during the build.

```
winget install Google.Protobuf      # Windows
brew install protobuf               # macOS
```

On Linux take the archive from github.com/protocolbuffers/protobuf (releases): Debian and Ubuntu
ship 3.21, too old. protoc elsewhere than `PATH`: set `PROTOC` to its full path. More in
`docs/platform.md`.

Cargo takes Platform (dashpay/platform at revision 575d961) and rust-dashcore from GitHub. On
Windows, Platform's repository has paths longer than 260 characters, and cargo unpacks it only
with long paths turned on in git, once per machine:

```
git config --global core.longpaths true
```

```
cargo build --release --bin evolite --bin evolite-renderer
```

The installer for Windows, Linux and macOS is built from the same workspace: `docs/installer.md`,
"Building a release".

## Running

```
cargo run --bin evolite                                # the real wallet, testnet; data in %APPDATA%\evolite
cargo run --bin evolite -- --data-dir <folder>         # another data folder
cargo run --bin evolite -- --network mainnet           # mainnet (its own folder, evolite-mainnet)
cargo run --bin evolite -- --theme light --lang en     # theme and language for this run
```

Flags of the app: `--data-dir <folder>`, `--network testnet|mainnet`, `--theme dark|light`,
`--lang <code>` (`ru`, `en`, `uk`, `de`, `es`, `fr`, `pt-BR`, `id`, `tr`, `vi`, `zh-Hans`, `ja`,
`ko`, `hi`, `ar`).

## Documentation

- `docs/development.md` — for working on evolite: build speed, the screenshot tools
  (`dev-tools`), tests, the crates and the interface code.
- `docs/platform.md` — the Platform and rust-dashcore stack, what the machine needs, trust in
  quorum keys.
- `docs/platform-wallet.md` — how the core uses platform-wallet: identities, credits, DashPay,
  NFTs.
- `docs/renderer.md` — the sandboxed picture renderer.
- `linux-check/` — building, testing and taking screenshots on Linux in Docker.
- `docs/installer.md` — the installer and uninstaller on Windows, Linux and macOS, and which
  architectures are built where (`.github/workflows/build.yml`).
