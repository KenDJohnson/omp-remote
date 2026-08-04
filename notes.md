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

### Questions for later

- None yet.
