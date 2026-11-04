# Development

For working on evolite: build speed, the screenshot tools, tests and how the code is laid out.
Building and running for a user is in the README. The design is in the Figma file
"evolite — UI/UX".

## Build speed

Debug info for dependencies is turned off and `rust-lld` does the linking
(`.cargo/config.toml`); it is worth excluding the `target` and `%USERPROFILE%\.cargo` folders
from Windows Defender scanning. `cargo run -p evolite-ui --example app_icon` rebuilds the app icon
from its SVG.

`--render-worker` is a flag of the app for itself: it starts itself with it as the sandboxed
picture renderer (`docs/renderer.md`).

## Screenshots and checks: the `dev-tools` feature

The tools for screenshots are left out of the release build; `--features dev-tools` puts them in.
Without it these flags are not read at all, and `--screen` or `--demo` start the usual wallet.

```
cargo run --features dev-tools -- --demo                             # the demo wallet with the example from the mockups
cargo run --features dev-tools -- --screen overview --theme light    # a screen from the mockups on the demo wallet
cargo run --features dev-tools -- --screen send --lang ar --screenshot send.png   # a screenshot, then exit
cargo run --features dev-tools -- --open activity --data-dir <folder> --screenshot a.png   # the real wallet, read only
```

- `--demo` — the demo core with the example from the mockups.
- `--screen <name>` — a screen in its mockup state; always on the demo wallet.
- `--open <name>` — the same screen over the real wallet in `--data-dir`, read only: nothing is
  sent that changes it, no settings are written. The vault opens with the password from
  `EVOLITE_SHOT_PASSWORD`; the shot waits for the sync and `EVOLITE_SHOT_SETTLE` seconds more
  (15 by default).
- `--screenshot <file.png>` — save the window once everything is drawn, then exit.
- `--window <W>x<H>` — the window size in points, for screenshots of small screens.

Screens: `welcome`, `overview`, `send`, `receive`, `activity`, `identity`, `create-identity`,
`contacts`, `shielded`, `apps`, `collection`, `settings`, `probe` (a check of writing systems);
with dialogs and states open — for example `send-confirm`, `activity-card`,
`identity-unfinished`, `add-contact`, `shield`, `connect`, `sign`, `gift`, `phrase`,
`appearance`, `about`, `report`, `wallets`, `wallet-menu`, `wallet-switch`; onboarding steps —
`create`, `verify`, `password`, `restore`, `dashpay`, `sync`. The full list is in
`crates/evolite-ui/src/cli.rs`.

Screenshots are not kept in git: the app takes them itself (`--screen … --screenshot …`).

## Tests

```
cargo t                                         # all tests: cargo test --workspace (an alias)
cargo t --features evolite-ui/dev-tools         # the same with the screenshot tools and the demo core
cargo test -p evolite-ui --no-default-features --features dev-tools   # only the interface on the demo wallet, seconds
cargo t live_real_send -- --ignored --nocapture # one live test on testnet
```

`--no-default-features` alone builds the demo-only app as well (the demo is its only core); with
`dev-tools` the tests of the flags and presets and the picture path through the renderer run too.

Run tests with `cargo t` (the whole workspace) and pick them by name: building a single package
(`-p`) enables a different set of features in its dependencies, and then almost the whole
Platform stack is rebuilt. Live tests (`#[ignore]`) go to testnet and take the phrase and the
names of test identities from `.env.test.local` (template: `.env.test.example`; the file never
goes into git). They run on a copy of a "warm" wallet in `%TEMP%\evolite-live-warm`: restoring the
phrase creates it once (about 10 minutes), and each test on a copy only catches up with new
blocks.

### What CI checks

`.github/workflows/ci.yml` runs on every pull request to `dev` or `master`, on every push to them
and by hand (`workflow_dispatch`). Each check is a command you can run here as it is; the Rust
release is the one in `rust-toolchain.toml` (rustup picks it up), so a new stable release does not
change what CI says. Live tests (`#[ignore]`) never run in CI.

Two checks, one per system, each running its commands in this order:

| Check | Commands |
|---|---|
| `linux` | `cargo fmt --all --check` |
| | `cargo clippy --workspace --all-targets --locked --features evolite-ui/dev-tools -- -D warnings` |
| | `cargo test --workspace --locked` |
| | `cargo test --workspace --locked --features evolite-ui/dev-tools` |
| | `cargo check -p evolite-ui --no-default-features --features dev-tools --locked` (in its own `CARGO_TARGET_DIR`: `-p` builds other features of the dependencies) |
| `windows` | `cargo test --workspace --locked` |
| | `cargo test --workspace --locked --features evolite-ui/dev-tools` |
| | `cargo clippy --workspace --all-targets --locked -- -D warnings` (the code under `cfg(windows)`) |

The code under `cfg(target_os = "macos")` is linted by neither.

The lints are set once for the workspace (`[workspace.lints]` in `Cargo.toml`): clippy's default
set, every warning an error in CI. When a lint is wrong for one place, allow it on that item only,
with the reason in it:

```rust
#[allow(
    clippy::too_many_arguments,
    reason = "the fields of the transition as Platform defines it; a struct would only rename them"
)]
fn sign_transition(/* … */) {}
```

A new Rust release is taken on purpose, in a small pull request of its own: `channel` in
`rust-toolchain.toml` (and the release in `linux-check/Dockerfile`), with the fixes its new
lints ask for.

## Architecture

Four crates in one workspace; the boundary between the interface and the wallet is the types in
`evolite-api`.

| Crate | What is inside | What is not there |
|---|---|---|
| `crates/evolite-api` | Commands (`Command`), answers (`Event`), a snapshot of everything the screens show (`Snapshot`). Amounts are in duffs, times in Unix seconds. The phrase and the password have their own types: they are never printed and are wiped (`zeroize`). | Logic, and a type for a private key: keys never leave the core. |
| `crates/evolite-core` | The wallet on its own thread. `Core::real` (`real/`) — dash-spv, storage (`vault.rs`), Platform through platform-wallet and dash-sdk (`chain.rs`), DashPay, identities, the pool, apps, NFTs, wallets (`wallets.rs`). `Core::mock` (`mock/`) — the demo with the example from the mockups. `diag` — the journal for problem reports. | egui. |
| `crates/evolite-render` | The renderer for untrusted pictures (SVG with SMIL and CSS animation, PNG, JPEG, GIF, WebP, GLB) in a separate sandboxed process — `docs/renderer.md`. | Keys, the network, the user's files. |
| `crates/evolite-ui` | Screens in egui (the `evolite` binary) and the renderer without window code (`evolite-renderer`, placed alongside). | Keys, the network, the wallet's files on disk. |

A screen receives `Ctx` — the latest snapshot, the time and a place for commands. Commands go to
the core after the frame, answers arrive as events and wake the interface; a frame never waits for
the network.

Features: `real` (default) — the real core; without it (`--no-default-features`) only the demo is
built, and everything builds in seconds. `pool` (default in `evolite-ui`) — the private pool:
Orchard proofs are a noticeable part of a clean build. `dev-tools` (`evolite-ui`, off by
default) — the screenshot flags, the mockup presets, the script probe and the demo core beside the
real one; the release and the installer are built without it. In `evolite-core` the demo core is
the `demo` feature, which `dev-tools` turns on.

### Interface

- egui / eframe **0.36** — the same line as Dash Evo Tool.
- `components/` — the UI kit from Figma: buttons, status badges, cards, fields, toggles,
  transaction rows, dialogs. Almost everything is drawn by hand with `Painter`.
- `screens/` — one file per screen; large ones get a folder (`onboarding/`, `settings/`).
- `i18n/` and `locales/` — localisation. The source language of the interface is English:
  strings are marked with the `tr!("…")` macro, and the English text is the key.
  `perl crates/evolite-ui/locales/extract.pl` collects the keys from the `tr!` marks into
  `locales/_keys.json`. The catalogs `locales/<code>.json` cover 14 languages, `ru.json` among
  them (there is no `en.json`); a test checks that every catalog covers every key. The languages'
  own names are in `locales/languages.json`. Plurals: `plural(n, tr!("payment"), tr!("payments"))`
  gives the key `"{} payment|{} payments"`, translated by CLDR plural category. Until a language
  is chosen the interface is in English; at start-up it takes the system language or the one
  from the settings. Arabic gets a mirrored frame (`rtl.rs`) and its own text order.
- `theme.rs` — colours from the Figma variables (Light / Dark); the Inter, JetBrains Mono,
  Noto Sans Arabic and Devanagari fonts ship with the app (`assets/fonts`, OFL licences alongside).
  Chinese, Japanese, Korean and the rarer Latin letters come from the system's fonts, found by
  file name in its font folders on Windows, macOS and Linux (Segoe UI / Helvetica / DejaVu,
  Microsoft YaHei / PingFang / Noto Sans CJK and the like; `theme::system_font`).
- `icons.rs` — icons and the logo from SVG (`assets/icons`), recoloured for the theme.
