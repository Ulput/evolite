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

## Carried over from upstream (SPV P0, customer's decision 2026-11-14)

Two upstream fixes, taken one by one: the whole copy can't move to v0.46.0, which doesn't build with Platform 575d961. Applied in this order after the shim; each patch is `git diff` against the one before, in the clone `C:\reps\deps\rust-dashcore`, branch `evolite/spv-p0-final`.

| Patch | Upstream | Why | What changed in carrying it over | When to remove |
|---|---|---|---|---|
| `evolite-mnlist-verify.patch` | `c9b3c0ec` (#1200) | A masternode list diff (MnListDiff, alone or inside QRInfo) is checked against its block's coinbase before it is applied: the coinbase merkle proof, then the masternode and quorum roots the coinbase commits to. | **Dropped:** `sml_engine/snapshot_codec.rs`, which belongs to upstream's newer engine storage (not here). **Adapted:** our storage replays stored messages (`storage/masternode.rs`), so the replay builds a validating engine too, and a stored message that fails is «does not apply» in the log and the lists are asked again. A test-only `without_validation()` serves the replay's mechanics tests on made-up messages. **Added tests:** a forged masternode, quorum or block hash in a single diff, and a forged diff inside the real mainnet QRInfo, are refused with the engine unchanged; the replay takes the real QRInfo and refuses it forged. | With Platform v5.1 (`c9aeac0` contains #1200). |
| `evolite-fork-validate.patch` | `086194ed` (#1304) | Received block headers are validated (continuity, proof of work, Dark Gravity Wave) against the stored block or segment tip they extend, **before** routing, so a fork is never detected or followed on headers nobody checked. | **Adapted:** upstream's surrounding code from the network rewrite (#902: the answered locator, request release) is not here. Its test runs on our `RequestSender` and our pipeline (a synced tip is `mark_tip_complete`). | When Platform takes a rust-dashcore with #1304. |
| `evolite-fork-ban.patch` | — (**evolite adaptation, not from upstream**) | #1304 also bans the peer that sent invalid headers; upstream does it through the #902 network, which isn't here. | `NetworkRequest::BanPeer(addr, reason)`, handled with the existing `PeerNetworkManager::ban_peer`; `handle_headers_pipeline` takes the sending peer (`msg.peer_address()`). A ban only for headers that fail validation, never for a timeout or a disconnect. Three filter tests bind every `NetworkRequest` variant: a ban is skipped there. **Tests:** a bad header bans its peer, a good one doesn't; on the 30 real mainnet headers a fork off 2000000 with its own proof of work but easier than DGW is refused, its peer banned, the tip kept. | With Platform v5.1 (`c9aeac0` contains #902): drop this patch, and apply `086194ed` as written. |

**Upstream tests:** those that came with the two commits run in the clone (where the data in `tests/` is):
- `dash-spv` library: 679 passed (1 ignored, as upstream), including `sml_engine`, `validation::masternode_list`, `sync::masternodes` and `sync::block_headers`;
- `dashcore`: `merkle_tree::block`, `sml::masternode_list` and `network::message_sml`, 29 passed.

**Not ported, needs #902:** upstream's own assertions on the #902 mock network (`MockNetworkManager::banned_peers`) and the `release_request` of the answered locator; their meaning is covered by the tests above.

**Copy = source + patches.** A clean `0dcdd823` with `evolite-shim.patch`, `evolite-mnlist-verify.patch`, `evolite-fork-validate.patch` and `evolite-fork-ban.patch` applied in this order gives the 12 crates here, `tests/` and `artifacts/` aside.
