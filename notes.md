# Notes

## 2026-08-04

### Decisions

- Keep `PLAN.md` local and uncommitted, per request.
- Follow the repository's Nix-first workflow; contributor commands run through `direnv exec .`.
- Treat Milestone 1 as a complete runtime boundary: transport reassembly stays in `omp-rpc`; process ownership, correlation, event fan-out, prompt state, and shutdown live in `omp-runtime`.
- `OmpRuntime` owns request IDs; untracked command frames are rejected so response correlation cannot be bypassed.
- The runtime negotiates protocol v2 when advertised and retains legacy-v1 compatibility when startup capabilities are absent.
- Runtime state uses `watch`; event delivery uses a bounded `broadcast` channel with a reserved initial receiver so startup events are not lost before the caller subscribes.
- Closing the runtime handle or calling `shutdown` closes stdin first, drains accepted responses while waiting, and kills only after the configured grace period.
- State revisions advance only for observable authoritative state changes; event sequences advance for both deltas and streamed OMP events.
- Subscription registration and snapshot/replay selection occur in one actor turn. A replay is accepted only when every event sequence is contiguous and every delta base reaches the current revision.
- Subscriber queues never block the actor. Overflow marks the subscription as requiring resynchronization before any buffered update can be consumed.
- Interaction leases use caller-supplied UTC milliseconds on the wire and Tokio monotonic deadlines for automatic expiry.
- Wire-visible state DTOs now live in `omp-control-protocol`; `omp-control-plane` consumes and re-exports them without a dependency in the opposite direction.
- Protocol versions are numeric newtypes so unknown versions decode and fail explicitly during hello negotiation rather than failing as malformed CBOR.
- Capabilities are deterministic string sets. Unknown additive capabilities can pass through without changing Rust enums.
- Mutating request envelopes require stable operation IDs; read-only requests reject them to keep deduplication semantics unambiguous.
- CBOR encoding writes through a bounded writer, so oversized frames fail without first allocating the oversized output.
- The browser-WASM test executes the same ping encoding against the native golden vector. `wasm-bindgen-test` is pinned to the version shipped by nixpkgs' `wasm-bindgen-cli`.
- SQLite stores explicit fixed-width columns for scopes/cursors/lifecycle plus CBOR only for typed operation outcomes; schema versioning uses `PRAGMA user_version`.
- Stable server IDs are generated once and loaded from metadata on every restart.
- Device tokens and pairing secrets are SHA-256 hashed before storage and compared in constant time. Random 256-bit pairing secrets use base64url only at the out-of-band boundary.
- Pending operations become indeterminate after daemon restart. They are never blindly re-executed, preventing duplicate prompts when execution may have completed before the outcome was persisted.
- Active processes become interrupted and lose volatile process/run IDs on startup; durable session resume metadata remains intact.
- Direct daemon TLS is restricted to TLS 1.3. Ordinary public and Tailscale-issued certificates share the trusted-certificate mode; native self-signed deployments carry a SHA-256 certificate fingerprint in the pairing bundle.
- Plain WebSockets are accepted only on an explicitly configured loopback development listener. Trusted reverse proxies must also connect through the exact configured loopback endpoint.
- Authentication is the first CBOR frame under a smaller pre-authentication limit. Authentication failures close without sending state or credential-specific diagnostics.
- Pairing credential exchange consumes the single-use secret and inserts the hashed per-device token in one SQLite transaction, so a crash cannot consume a secret without issuing its device record.
- Pairing creation is available only through a `0600` Unix socket. QR and browser payloads use URL fragments; the browser link therefore does not transmit the pairing secret in the HTTP request.
- UI interaction request/response envelopes carry the agent ID, and responses additionally carry the lease holder ID so the daemon can enforce ownership before forwarding to OMP.
- Network delivery uses bounded priority lanes. Subscription tasks use non-blocking enqueue and emit an explicit replay gap on overflow, keeping control-plane actors independent of socket backpressure.
- Persisted agent/session cursors seed restored actor snapshots. Since replay buffers are intentionally volatile, any client behind a restored cursor receives a snapshot/resynchronization path rather than fabricated replay.
- The daemon controller now owns runtime launch, prompt, steering, abort, session switch, and graceful shutdown wiring; stable operation outcomes wrap mutating dispatch before execution.
- The control client separates transport (`WebSocketAdapter`/`BinaryWebSocket`), credential persistence, replicated state, and the reconnecting runner so Dioxus platform shells can supply only platform-specific adapters and storage.
- A pairing welcome is not considered successful until the issued device credential is durably saved; subsequent reconnects authenticate with that device credential.
- Every reconnect assigns fresh connection-local request IDs while retaining the stable operation ID for mutating requests and UI responses. Read-only requests are failed on disconnect rather than retried ambiguously.
- The replicated reducer marks an agent as requiring resynchronization after any revision or event-sequence failure, suppresses its resume cursor, and rejects further incremental updates until a fresh snapshot arrives.
- Browser clients reject certificate fingerprints because Web APIs do not expose peer certificates. Native clients implement fingerprint pinning with Rustls signature verification delegated to WebPKI.

### Questions for later

- None yet.
