# CLAUDE.md — CommitLabs Contracts

This file documents the codebase structure, development workflows, and conventions for AI assistants working on this repository.

---

## Project Overview

CommitLabs Contracts is a collection of **Stellar Soroban smart contracts** written in **Rust** implementing a liquidity commitment protocol. Users lock assets into time-bound commitments represented as NFTs; contracts manage the full lifecycle (creation → attestation → settlement/early exit), fee collection, allocation strategies, and marketplace operations.

**Tech stack:**
- Language: Rust (edition 2021, `#![no_std]` for all contract crates)
- Blockchain: Stellar Soroban (`soroban-sdk = "21.0.0"`)
- Compile target: `wasm32v1-none` (release/deploy), native (tests)
- Build system: Cargo workspace
- CI: GitHub Actions (`.github/workflows/`)

---

## Repository Structure

```
.
├── Cargo.toml                  # Workspace root — lists all member crates
├── Cargo.lock
├── contracts/                  # All smart contract crates
│   ├── shared_utils/           # Shared library (not a deployable contract)
│   ├── commitment_core/        # Core commitment lifecycle
│   ├── commitment_nft/         # NFT representation of commitments
│   ├── attestation_engine/     # Commitment health verification
│   ├── allocation_logic/       # Pool allocation strategies
│   ├── commitment_marketplace/ # Marketplace for commitments
│   ├── commitment_transformation/ # Tranche creation from commitments
│   ├── commitment_interface/   # Shared interface types
│   ├── price_oracle/           # Price oracle contract
│   ├── mock_oracle/            # Mock oracle for testing
│   ├── time_lock/              # Time-locked operations
│   └── version-system/         # Contract upgrade versioning
├── tests/
│   └── integration/            # Cross-contract integration tests (excluded from workspace)
├── scripts/
│   ├── build-contracts.sh      # Build all contracts to WASM
│   ├── deploy.sh               # Core deploy logic (called by testnet/mainnet scripts)
│   ├── deploy-testnet.sh       # Deploy to Stellar testnet
│   ├── deploy-mainnet.sh       # Deploy to Stellar mainnet
│   ├── generate-docs.sh        # Generate and copy rustdoc to docs/
│   └── benchmark.sh            # Run benchmarks
├── config/
│   └── deploy.env.example      # Template for deployment environment variables
├── deployments/
│   ├── testnet.json            # Deployed contract addresses (testnet)
│   └── mainnet.json            # Deployed contract addresses (mainnet)
├── docs/                       # Architecture and protocol documentation
│   ├── ARCHITECTURE.md
│   ├── FEES.md
│   ├── CONTRACT_FUNCTIONS.md
│   ├── SECURITY_*.md
│   ├── TEST_COVERAGE.md
│   └── ...
└── benchmarks/                 # Benchmark results
```

---

## Contracts

### `shared_utils` — Shared Library

Not deployed independently. Imported as a path dependency by all contracts.

| Module | Purpose |
|---|---|
| `math.rs` | `SafeMath` — checked add/sub/mul/div, percent helpers |
| `fees.rs` | `fee_from_bps`, `net_after_fee_bps`, `BPS_SCALE = 10000` |
| `access_control.rs` | `AccessControl::require_admin`, `require_admin_or_authorized`, `is_admin` |
| `storage.rs` | `Storage::get_admin`, `is_initialized`, `set_initialized`, `require_initialized` |
| `validation.rs` | Input validation helpers |
| `rate_limiting.rs` | `RateLimiter` — per-address, per-function fixed-window rate limiting |
| `pausable.rs` | `Pausable` — pause/unpause pattern |
| `emergency.rs` | `EmergencyControl` — emergency stop mechanisms |
| `events.rs` | `emit_error_event` and other event helpers |
| `errors.rs` | `ErrorHelper` — log/panic helpers |
| `error_codes.rs` | Shared error code constants |
| `batch.rs` | Batch operation utilities |
| `time.rs` | `TimeUtils` — timestamp and duration helpers |

### `commitment_core`

Manages commitment lifecycle: creation, value tracking, settlement, early exit.

Key types: `Commitment`, `CommitmentRules`, `CommitmentCreatedEvent`

Commitment statuses (stored as `String`): `"active"`, `"settled"`, `"violated"`, `"early_exit"`

Commitment types (stored as `String`): `"safe"`, `"balanced"`, `"aggressive"`

Storage: **instance storage** for commitments, owner lists, admin, counters, reentrancy guard.

### `commitment_nft`

Stores NFT metadata and ownership for each commitment. Each NFT maps to a `CommitmentNFT` with `CommitmentMetadata`.

Key types: `CommitmentNFT`, `CommitmentMetadata`, `TransferParams`

Storage: **persistent storage** for NFTs and ownership; instance storage for admin and counters.

### `attestation_engine`

Records attestations and health metrics for commitments. Calls `commitment_core` cross-contract to read commitment data.

Attestation types (stored as `String`): `"health_check"`, `"violation"`, `"fee_generation"`, `"drawdown"`

Storage: **persistent storage** for attestations and health metrics; instance storage for analytics counters and admin.

### `allocation_logic`

Registers pools and manages allocation/rebalancing of commitment amounts across strategies.

Storage: **persistent storage** for pools/allocations; instance storage for admin and pool registry.

### Other Contracts

- `commitment_marketplace` — marketplace listings/trades
- `commitment_transformation` — splits commitments into tranches, collects transformation fee
- `price_oracle` / `mock_oracle` — price feed contracts
- `time_lock` — time-locked admin operations
- `version-system` — WASM hash and migration tracking
- `commitment_interface` — shared interface types (error/type definitions)

---

## Build Commands

```bash
# Build all workspace contracts for WASM (deploy target)
cargo build --workspace --target wasm32v1-none --release

# Build using the provided script (validates artifacts exist after build)
bash scripts/build-contracts.sh

# Build a single contract
cargo build -p commitment_core --target wasm32v1-none --release

# Build for native (needed for tests — do not add --target here)
cargo build --workspace
```

**WASM target note:** The primary deploy target is `wasm32v1-none`. CI also uses `wasm32-unknown-unknown` for compatibility checks. Never use `--target wasm32v1-none` when running tests (use native).

---

## Testing

### Unit Tests

Each contract has `src/tests.rs`. Run on the native target (no `--target` flag).

```bash
# Run all workspace unit tests
cargo test --workspace

# Run tests for a specific contract
cargo test -p commitment_core
cargo test -p commitment_nft
cargo test -p attestation_engine
cargo test -p allocation_logic
cargo test -p shared_utils

# Run with stdout output
cargo test --workspace -- --nocapture

# Run a specific test by name
cargo test -p commitment_core test_initialize
```

### Integration Tests

The `tests/integration/` package is **excluded from the workspace** (see root `Cargo.toml` `exclude` list). It must be run separately:

```bash
cd tests/integration
cargo test
```

This package depends on: `commitment_nft`, `commitment_core`, `attestation_engine`, `price_oracle`.

### Benchmark Tests

```bash
# Run benchmarks (uses the "benchmark" feature flag)
cargo test --workspace --features benchmark --release -- --nocapture
```

### Test Conventions

- Tests live in `src/tests.rs` with `#[cfg(test)]` and `use super::*;`
- Use `soroban_sdk::testutils` features: `Address as _`, `Events`, `Ledger`
- Register contracts with `e.register_contract(None, ContractStruct)`
- Call contract functions via `e.as_contract(&contract_id, || { ... })`
- `Env::default()` for test environment; set ledger timestamps via `e.ledger().with_mut(|l| l.timestamp = ...)`
- Mock auth for testing: `e.mock_all_auths()`

---

## Deployment

### Setup

```bash
cp config/deploy.env.example config/deploy.env
# Edit config/deploy.env — set STELLAR_ACCOUNT and STELLAR_ADMIN_ADDRESS
```

Required environment variables:
- `STELLAR_ACCOUNT` — deployer secret key or identity name
- `STELLAR_ADMIN_ADDRESS` — public key for contract admin initialization

### Deploy

```bash
bash scripts/deploy-testnet.sh    # Deploy to Stellar testnet
bash scripts/deploy-mainnet.sh    # Deploy to Stellar mainnet
```

### Deployment Order

Contracts **must** be deployed in this order (enforced by deploy script):
1. `commitment_nft`
2. `commitment_core`
3. `attestation_engine`

`allocation_logic` is deployed independently. Other contracts have no fixed ordering constraints.

### Post-Deployment Initialization

The deploy script performs these initialization steps automatically:
1. `commitment_nft.initialize(admin)`
2. `commitment_core.initialize(admin, nft_contract_id)`
3. `commitment_nft.set_core_contract(commitment_core_id)`
4. `commitment_nft.add_minter(commitment_core_id)`
5. `attestation_engine.initialize(admin, commitment_core_id)`

Contract addresses are written to `deployments/testnet.json` or `deployments/mainnet.json`.

---

## Fee System

All percentage-based fees use **basis points (bps)**: `10000 bps = 100%`.

Use `shared_utils::fees`:
```rust
use shared_utils::{fee_from_bps, net_after_fee_bps, BPS_SCALE};

let fee = fee_from_bps(amount, fee_bps);         // (amount * bps) / 10000
let net = net_after_fee_bps(amount, fee_bps);    // amount - fee
```

| Fee Type | Contract | Trigger |
|---|---|---|
| Creation fee | `commitment_core` | `create_commitment` — basis points of commitment amount |
| Early exit fee | `commitment_core` | `early_exit` — penalty percent from commitment rules |
| Attestation fee | `attestation_engine` | `attest` — fixed amount per attestation |
| Transformation fee | `commitment_transformation` | `create_tranches` — basis points of total value |

Admin functions per contract: `set_creation_fee_bps`, `set_fee_recipient`, `withdraw_fees(asset, amount)`.

Collected fees are stored per asset under `CollectedFees(Address)` in each contract.

---

## Code Conventions

### Error Handling

All contracts define a `#[contracterror]` enum with `#[repr(u32)]`. Error variants have explicit integer codes. Contracts emit an error event via `emit_error_event(e, code, context)` then `panic!` with the human-readable message.

```rust
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum MyError {
    NotInitialized = 1,
    Unauthorized = 2,
    // ...
}
```

Use a local `fail()` helper pattern:
```rust
fn fail(e: &Env, err: MyError, context: &str) -> ! {
    emit_error_event(e, err as u32, context);
    panic!("{}", err.message());
}
```

### Storage Patterns

- **Instance storage** (`e.storage().instance()`) — for frequently-accessed singletons (admin, config, counters, reentrancy guard). Cheaper but has size limits.
- **Persistent storage** (`e.storage().persistent()`) — for per-entity data (NFTs, attestations, pools). Has separate size limits per entry.

Storage keys are defined as `#[contracttype] enum DataKey { ... }`.

### Access Control

Always call `caller.require_auth()` before checking authorization. Use `shared_utils::AccessControl::require_admin` for admin-gated functions.

### Reentrancy Guard

Use instance storage to set/clear a reentrancy flag around functions that make external calls (token transfers, cross-contract calls). Follow checks-effects-interactions order.

### Math

Never use raw arithmetic. Use `SafeMath` or Rust's `.checked_add()` / `.checked_sub()` / `.checked_mul()` / `.checked_div()` with `.expect("context")`.

### `no_std`

All contract crates are `#![no_std]`. Do not use `std` types. Use `soroban_sdk::String`, `soroban_sdk::Vec`, `soroban_sdk::Map` instead of their std equivalents.

### Doc Comments

Use `///` for public items and `//!` for module-level docs. The `generate-docs.sh` script runs `cargo doc --workspace --no-deps` and copies output to `docs/`.

---

## Security Patterns

- **Auth:** `caller.require_auth()` must precede any admin check.
- **Reentrancy:** Guard using instance storage bool; clear before external call returns.
- **Overflow:** Release profile sets `overflow-checks = true`; still use checked arithmetic.
- **Input validation:** Validate all external inputs at contract entry points. Use `shared_utils::Validation`.
- **Rate limiting:** `shared_utils::RateLimiter` is available. Rate limits are disabled until configured by admin.
- **Pausable:** Contracts can implement `Pausable` from `shared_utils` for emergency halts.

Known limitations are documented in `docs/KNOWN_LIMITATIONS.md`.

---

## CI/CD

GitHub Actions workflow: `.github/workflows/` — runs on push/PR to `master`.

Pipeline steps:
1. Install Rust stable + `wasm32-unknown-unknown` target
2. Install Stellar CLI via Homebrew
3. Cache Cargo dependencies
4. Build all workspace contracts (WASM): `cargo build --target wasm32-unknown-unknown --release`
5. Build each contract with Stellar CLI: `stellar contract build` (per `contracts/*/`)
6. Run unit tests: `cargo test --workspace`
7. Run integration tests: `cd tests/integration && cargo test`
8. Run benchmarks: `cargo test --workspace --features benchmark --release`
9. Verify WASM artifacts exist in `target/`

CI runs on `macos-latest`.

---

## Documentation

| File | Contents |
|---|---|
| `README.md` | Project overview, quick-start commands |
| `docs/ARCHITECTURE.md` | Component diagram, storage layout, core flows |
| `docs/FEES.md` | Fee types, basis points, collection and withdrawal |
| `docs/CONTRACT_FUNCTIONS.md` | Function-level API reference |
| `docs/SECURITY_CONSIDERATIONS.md` | Access control, reentrancy, overflow, known issues |
| `docs/SECURITY_CHECKLIST.md` | Audit preparation checklist |
| `docs/THREAT_MODEL.md` | Threat model |
| `docs/KNOWN_LIMITATIONS.md` | Current gaps (auth, cross-contract validation) |
| `docs/TEST_COVERAGE.md` | Test coverage summary |
| `docs/UPGRADES.md` / `UPGRADE_PATHS.md` | Contract upgrade process |
| `DEPLOYMENT.md` | Detailed deployment guide |
| `TESTING.md` | Testing guide |
| `PERFORMANCE.md` | Performance benchmarks |

Generate API docs from Rust doc comments:
```bash
bash scripts/generate-docs.sh
# Output: docs/<crate>/index.html
```

---

## Common Workflows

### Add a function to an existing contract

1. Read the contract's `src/lib.rs` to understand the existing structure.
2. Add the function to the `#[contractimpl]` block.
3. Define any new storage keys in the `DataKey` enum.
4. Use `SafeMath` for arithmetic, `AccessControl::require_admin` for admin gates, `caller.require_auth()` for auth.
5. Add tests in `src/tests.rs`.
6. Run `cargo test -p <contract_name>` to verify.

### Add a new shared utility

1. Add a new module file in `contracts/shared_utils/src/`.
2. Declare it with `pub mod` in `contracts/shared_utils/src/lib.rs`.
3. Add a `pub use` re-export in `lib.rs` if it should be part of the public API.
4. Add tests in `contracts/shared_utils/src/tests.rs`.

### Deploy a new contract

1. Create crate under `contracts/<name>/` with `Cargo.toml` using `crate-type = ["cdylib", "rlib"]` and `soroban-sdk = "21.0.0"`.
2. Add to workspace `members` in root `Cargo.toml`.
3. Implement with `#[contract]`, `#[contractimpl]`, `#[contracttype]` etc.
4. Add deploy step to `scripts/deploy.sh`.

### Run full local validation

```bash
# Unit tests
cargo test --workspace

# Integration tests
cd tests/integration && cargo test && cd ../..

# Build WASM
bash scripts/build-contracts.sh

# Generate docs
bash scripts/generate-docs.sh
```
