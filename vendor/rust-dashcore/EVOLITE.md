# rust-dashcore, vendored for evolite (FEAT-0037)

**Source:** [dashpay/rust-dashcore](https://github.com/dashpay/rust-dashcore) at `0dcdd823` ("feat(dash-spv)!: validate header difficulty against Dark Gravity Wave", #1161).

**Why:** this commit is the one that checks header difficulty against Dark Gravity Wave. Before it, past the last checkpoint, a peer could serve a chain of minimal difficulty. That matters on mainnet. Platform 575d961, which evolite builds on, still pins `aca0202`. Our root `Cargo.toml` uses `[patch."https://github.com/dashpay/rust-dashcore"]` to point all 12 crates, ours and Platform's, at this copy.

## What is here

- The root `Cargo.toml`, with `members` cut down to the 12 crates below, so cargo can read the workspace. Our root `Cargo.toml` excludes `vendor/rust-dashcore`.
- `LICENSE`.
- The crates `dash`, `crypto`, `hashes`, `internals`, `dash-network`, `dash-network-seeds`, `dash-spv`, `key-wallet`, `key-wallet-manager`, `git-state`, `rpc-client` and `rpc-json`.

## What was removed

- `.git`.
- Every crate's `tests/` folder and `dash/artifacts/`. Their data is read only by rust-dashcore's own tests: `include_bytes!` and `include_str!` under `#[cfg(test)]` or the `test-utils` feature, which evolite does not enable. We don't run those tests.
- The crates `fuzz`, `dash-spv-ffi`, `key-wallet-ffi`, `ffi-c-tests`, `rpc-integration-test`, `dash-spv-bench` and `masternode-seeds-fetcher`.
- Files at the root other than `Cargo.toml` and `LICENSE`.

`benches/` and `examples/` stay: the crates' manifests declare them, and cargo reads the declarations.

## What was changed

**One shim**, kept as its own file: [`evolite-shim.patch`](evolite-shim.patch). It is `git diff 0dcdd823` of `dash-spv/src/chain/checkpoints.rs`, and it only adds:

- `Checkpoint::height` becomes `pub`;
- `CheckpointManager::last_checkpoint()` returns the highest checkpoint, as in `aca0202`.

platform-wallet 575d961 (`wallet_lifecycle.rs`) still calls both. rust-dashcore #375 removed them. The birth height logic is unchanged: by default the header tip, otherwise the last checkpoint.

To check that the copy matches the source plus the shim, apply `evolite-shim.patch` to a clean `0dcdd823` checkout. The 12 crates' source must then be identical.

## When to remove it

When Platform ships with rust-dashcore ≥ `0dcdd823` (v5.1.0), remove this folder, the `[patch]` section and the `exclude` from our root `Cargo.toml`.
