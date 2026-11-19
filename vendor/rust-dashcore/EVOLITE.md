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

## evolite's own: fork handling (SPV item 1, plan 2026-11-14 §1)

Applied after the four above. `git diff evolite/spv-p0-final` in a clone of our own, `C:\reps\deps\rust-dashcore-fork`, branch `evolite/spv-fork-work` (`C:\reps\deps\rust-dashcore` belongs to another session).

| Patch | Upstream | Why | What it does | When to remove |
|---|---|---|---|---|
| `evolite-fork.patch` | — (**evolite, not from upstream**) | A valid branch was followed, and our chain trimmed, as soon as it branched off below the tip, whatever its work. A peer could make us drop blocks with one cheap header. | **Work decides:** a fork is followed (`ForkDetected`, then the trim and resync) only when its headers have strictly more proof of work (`ChainWork`) than our stored chain from the fork point to the tip. On equal work we stay on the chain seen first, as Dash Core does. **Held until then:** the branch stays in memory (`BlockHeadersManager::fork`), nothing trimmed, and its peer is asked for what follows (`getheaders` from the branch's tip). Headers that continue it are validated against stored ancestors plus the branch (continuity, PoW, DGW), then added and weighed again. At most `MAX_FORK_HEADERS` = 2016: beyond that the branch is dropped and its peer disconnected (`NetworkRequest::DisconnectPeer`, no ban: its headers were valid). **Final blocks:** the checkpoint and ChainLock rule is checked again when weighing, so a ChainLock that came since drops the branch. **The ban:** a peer whose fork headers fail validation, first or continuing, is disconnected and banned for an hour (`ChangeReason::InvalidHeaders`, `ban_duration`); a manual ban keeps its day. **Tests:** bad PoW and easier difficulty refused with the peer banned, first headers and continuing a held branch (AC-T1.1, 1.2; the mainnet DGW case of `evolite-fork-ban.patch` stays); less or equal work held, not followed, the peer asked for more (1.3); more work followed in one message or grown over several, and work rather than length decides (1.4); a ChainLock arriving under a held branch drops it, the existing checkpoint and ChainLock cases stay (1.5); too long without more work, dropped and disconnected; the hour's ban. Eight mutations, each red. **SPV item 1б (in the same patch): after a block race we noticed nothing.** If we stored N₁ while the network went on N₂ → N+1, every announcement had a parent we didn't have and was dropped, and the tip froze until a restart. Now, synced: headers that extend nothing we store, and a ChainLock (`CLSig`) for a block we don't store, send `getheaders` with a **block locator** as Dash Core builds one (the tip, the nine before it, then steps doubling back, the lowest stored header last). For a ChainLock it goes to the peer that sent it and one other. At most once per peer per `LOCATOR_ASK_INTERVAL` (30 s). The peer answers from our common ancestor, and the branch is weighed as above. The tip segment's fallback getheaders (stale `inv` announcements, a peer ahead of us, a resumed sync, announcements during sync) carried the tip's hash alone, which a peer on another branch answers from genesis; it now carries the locator too (`reset_tip_with_locator`). Our part's work is kept in the held branch until our tip moves (review note). **Tests:** the locator's shape; a block race asked for and followed (AC-T1б.1); a ChainLock for an unknown block, our own, and while syncing (1б.2); a stream of unconnecting announcements asks each peer once per interval (1б.3); the fallback carries the locator; our work read again when the tip moves. Eight more mutations, each red. | When upstream compares chain work before following a fork. |

## evolite's own: peers over subnets (SPV item 2b, plan 2026-11-14 §2б)

Applied after `evolite-fork.patch` (items 1 and 1б): `git diff evolite/spv-fork-locator` in our own clone, `C:\reps\deps\rust-dashcore-fork`, branch `evolite/spv-peers-on-1b`.

| Patch | Upstream | Why | What it does | When to remove |
|---|---|---|---|---|
| `evolite-peers.patch` | — (**evolite, not from upstream**) | With our `max_peers` = 8 (evolite-chain's `client_config`; dash-spv's default is 3), one operator holding a block of addresses could still fill every connection and eclipse us. | **The subnet rule:** at most two outbound peers, connected or being connected, from one /16 (IPv4) or /32 (IPv6); an IPv4-mapped IPv6 address counts as IPv4 (`network::diversity`). **One gate:** `PeerPool::mark_connecting_within_subnets` checks and marks under one lock. Every outbound connect goes through `connect_to_peer` (known addresses, DNS fallback), which now says whether it started. The top-up loops count only connects that started, so a refused address leaves its place to the next. **Exempt:** peers the user configured (exclusive mode, «Own node»). **Tests:** five in one /16 give two (`pick_diverse`, AC-T2б.2), the pool's gate with a peer leaving, the manager's `connect_to_peer` in both modes. Six mutations, each red. | When upstream limits outbound peers per network group. |

## evolite's own: peers from our verified masternode list (SPV item 2в, plan 2026-11-14 §2б, AC-T2б.3)

Applied after `evolite-peers.patch`: `git diff evolite/spv-peers-on-1b` in our own clone, `C:\reps\deps\rust-dashcore-fork`, branch `evolite/spv-sml-peers`.

| Patch | Upstream | Why | What it does | When to remove |
|---|---|---|---|---|
| `evolite-sml-peers.patch` | — (**evolite, not from upstream**) | Our 8 peers came only from DNS seeds and other peers' `addr`. The subnet rule helps only partly against an eclipse from cheap addresses spread over subnets; a masternode put a collateral up, so eclipsing us with masternodes is costly. | **The hook:** `MasternodeListEngine::verified_peer_addresses` gives the Core P2P addresses of the valid masternodes in the newest list, and **none** for an engine that does not check diffs against their block's coinbase (`evolite-mnlist-verify`). **Four:** once a run, when the first list is in (`MasternodeStateUpdated`), the masternode manager picks `SML_PEERS` = 4 at random with at most two from one subnet (`diversity::pick_sml_peers`) and sends `NetworkRequest::SmlPeers`. **Kept:** the network manager saves them to `sml_peers.json` in the storage folder and reads them at the next start, until that run's list offers its own. **Tried first:** a top-up connects them before the known addresses, through the same subnet gate. The user's own peers (exclusive mode) are left alone. **Tests:** a verified list offers four valid ones over subnets, an unverified one none (and its candidates are none); offered once a run, not at every diff; the pick over subnets, fewer when there are fewer; kept, saved, read again and tried first. Seven mutations, each red. | When upstream takes peers from the masternode list. |

## evolite's own: a finalized transaction's output that became ours (BUG-232)

Applied after `evolite-sml-peers.patch`: `git diff evolite/spv-sml-peers` in a worktree of our own clone, `C:\reps\deps\rust-dashcore-bug232`, branch `evolite/bug232-finalized-outputs`.

| Patch | Upstream | Why | What it does | When to remove |
|---|---|---|---|---|
| `evolite-finalized-outputs.patch` | — (**evolite**; draft `docs/upstream/U5-finalized-tx-new-outputs.md`, not published) | A restored wallet read a transaction with a change pool too short for its change, so the change counted as paid out. Once the pool takes the address in and a rescan reads the block again, `confirm_transaction` returns at once for a chain-locked record, so the change never got its coin and the balance stayed short. | **In `ManagedCoreFundsAccount::confirm_transaction`:** at the finalized early return, `has_uncredited_output` looks for an output to an involved address whose outpoint is in none of `utxos`, the spent outpoints, `observed_spent`, `spent_before_funded`, `claim_guarded_outputs` or `spent_claims`. If there is one, `update_utxos` runs with the same arguments, applying its own rules, and the record is returned unchanged. Otherwise it returns `None`, as before. **Changed upstream test:** `test_rescan_recovery_does_not_reach_a_finalized_funding_transaction` documented that boundary. Renamed `…_reaches_…`: the coin an abandoned spend freed now comes back. **Tests** (`tests/finalized_new_outputs_tests.rs`): credited, with `updated_records` under `keep-finalized-transactions`; a spend read first gives no coin; nothing new gives `None`, no record, no revision bump. The mutant with the old early return fails two of them. key-wallet lib: 741 passed with the feature, 743 without. | When upstream credits outputs that became ours in a finalized transaction. |

**Upstream tests:** those that came with the two commits run in the clone (where the data in `tests/` is):
- `dash-spv` library: 679 passed (1 ignored, as upstream); with `evolite-fork.patch` 700 passed (694 with item 1, 700 with 1б), with `evolite-peers.patch` on top 706, with `evolite-sml-peers.patch` 711, in `rust-dashcore-fork`, including `sml_engine`, `validation::masternode_list`, `sync::masternodes` and `sync::block_headers`;
- `dashcore`: `merkle_tree::block`, `sml::masternode_list` and `network::message_sml`, 29 passed.

**Not ported, needs #902:** upstream's own assertions on the #902 mock network (`MockNetworkManager::banned_peers`) and the `release_request` of the answered locator; their meaning is covered by the tests above.

**Copy = source + patches.** A clean `0dcdd823` with `evolite-shim.patch`, `evolite-mnlist-verify.patch`, `evolite-fork-validate.patch`, `evolite-fork-ban.patch`, `evolite-fork.patch`, `evolite-peers.patch`, `evolite-sml-peers.patch` and `evolite-finalized-outputs.patch` applied in this order gives the 12 crates here, `tests/` and `artifacts/` aside.
