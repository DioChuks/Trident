# Trident reference contracts

Reference Soroban contracts used by Trident's own test suite (issue #258) —
they exist to give the indexer and the E2E suite (issue #268) a
deterministic, first-party source of on-chain events, distinct from the
`explorer/` frontend which only *reads* Trident's API.

This is its own Cargo workspace, separate from the repository root
workspace: contracts build for `wasm32v1-none` with a `panic = "abort"`
release profile, which is a different shape than the native `crates/`
services and would otherwise leak into their build settings.

## Contracts

| Contract  | Purpose                                                                 |
|-----------|--------------------------------------------------------------------------|
| `counter` | Smallest possible contract — one `u32` in storage. Deploy/invoke smoke test. |
| `token`   | Reference SEP-41 fungible token. Emits `transfer`/`mint`/`burn`/`clawback`/`approve` events shaped exactly as `crates/indexer/src/parser/token_events.rs` decodes them, and exposes `name()`/`symbol()`/`decimals()` for the token metadata resolver (issue #263). |
| `nft`     | Minimal non-fungible token — sequential ids, single owner, no approvals. |
| `escrow`  | Minimal three-party escrow (depositor/beneficiary/arbiter) over a SEP-41 token. |

Each is deliberately small; further depth (full SEP-41 compliance, NFT
approvals/metadata, escrow timeouts) is tracked in separate issues (#259,
#275, #277) and out of scope here.

## Toolchain

- Rust stable, `wasm32v1-none` target (`rustup target add wasm32v1-none`).
- [`stellar` CLI](https://github.com/stellar/stellar-cli) — version pinned in
  `.github/workflows/ci.yml` (`STELLAR_CLI_VERSION`), currently 25.2.0.
- `soroban-sdk` version is pinned via `[workspace.dependencies]` in this
  directory's `Cargo.toml` (`=26.1.0`) so every contract builds against the
  same SDK/protocol version.

## Building

```sh
cd contracts
cargo test --workspace                                    # native unit tests
cargo build --release --target wasm32v1-none --workspace  # WASM (all contracts)
# or, via the stellar CLI (also runs the optimizer):
stellar contract build
```

CI (`contracts` job) runs both steps on every push/PR — build-only, nothing
is deployed there. The `e2e-contract-events` job deploys the `token`
contract to a local Soroban network and asserts the emitted event is indexed
end to end.
