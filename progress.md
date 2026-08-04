# Progress

## 2026-08-04

- Committed the pre-existing `omp-rpc` wire-type work as `97aa7fe` (`Add OMP RPC protocol types`).
- Completed Milestone 1:
  - Added strict protocol-v2 chunk reassembly to `omp-rpc`, including ordering, interruption, byte-limit, UTF-8, and JSON validation.
  - Added `omp-runtime` with child supervision, automatic v2 negotiation, bounded JSONL reads, request correlation, event delivery, prompt lifecycle tracking, and graceful shutdown with forced-kill fallback.
  - Added the deterministic `omp-runtime-fixture` process and four end-to-end runtime tests.
  - Verified with `cargo test -p omp-rpc`, `cargo test -p omp-runtime`, and warning-free `cargo clippy -p omp-runtime --all-targets -- -D warnings`.
- Completed Milestone 2:
  - Added `omp-control-plane` with stable IDs, lifecycle/run/session snapshots, independent state and event cursors, and exhaustive typed deltas.
  - Added atomic actor subscriptions, contiguous bounded replay, explicit resynchronization on replay or subscriber gaps, and an exclusive expiring interaction lease.
  - Verified deterministic reducer convergence, identical subscriber ordering, missed-revision rejection, event-only cursor advancement, lease expiry, and slow-subscriber resynchronization in six tests.
  - Verified with `cargo test -p omp-control-plane` and warning-free `cargo clippy -p omp-control-plane --all-targets -- -D warnings`.
- Completed Milestone 3:
  - Added `omp-control-protocol` with versioned client/server frames, typed control envelopes, shared state DTOs, authentication hello types, capability negotiation, and operation-ID validation.
  - Added bounded single-frame CBOR encoding/decoding with distinct pre-auth and authenticated limits plus trailing-frame rejection.
  - Added a fixed CBOR compatibility vector, optional-field compatibility coverage, unsupported-version rejection, and matching native/browser-WASM encoding tests.
  - Added a Nix flake WASM protocol check and the pinned flake-provided linker/test runner required to execute it.
  - Verified native protocol/control-plane tests, warning-free clippy, `wasm32-unknown-unknown` compilation, and the executed WASM golden-vector test.
- Milestone 4 is next: SQLite metadata, credentials, pairing records, and operation idempotency.
