# OMP Remote

OMP Remote is a secure remote-control surface for [Oh My Pi](https://github.com/can1357/oh-my-pi) agents. A local daemon supervises OMP RPC processes, exposes their state over an authenticated WebSocket, and lets paired desktop or web clients follow runs and act on them.

## What it provides

- One daemon-managed OMP process per agent ID.
- Live transcripts for prompts, assistant output, thinking, tool activity, and notices.
- Prompt, steer, follow-up, abort, stop, and session-switch controls.
- Exclusive leases for answering interactive agent questions from one client at a time.
- One-time pairing links, per-device credentials and scopes, and device revocation.
- Reconnecting clients with state snapshots, deltas, and bounded event replay.
- Direct TLS, certificate-pinned self-signed TLS, or loopback-only reverse-proxy and development modes.

## Repository layout

| Path | Purpose |
| --- | --- |
| `crates/ompd` | Daemon, WebSocket service, local admin socket, pairing, and SQLite persistence |
| `crates/omp-app` | Shared Dioxus desktop/web user interface |
| `crates/omp-control-client` | Reconnecting native and browser control client |
| `crates/omp-control-plane` | Authoritative per-agent state, subscriptions, replay, and interaction leases |
| `crates/omp-control-protocol` | Versioned CBOR protocol shared by the daemon and clients |
| `crates/omp-runtime` | Supervision of `omp --mode rpc` child processes |
| `crates/omp-rpc` | Typed representations of the OMP JSONL RPC protocol |

See [Architecture](docs/architecture.md) for the component and data-flow details.

## Quick start

Development is Nix-first. Install Nix and direnv, make an OMP executable available, then enter the repository environment:

```sh
direnv allow
direnv exec . cargo build --workspace
```

Start a loopback-only development daemon:

```sh
mkdir -p /tmp/omp-remote-dev
direnv exec . cargo run -p ompd -- serve \
  --database /tmp/omp-remote-dev/ompd.sqlite3 \
  --admin-socket /tmp/omp-remote-dev/ompd.sock \
  --omp /absolute/path/to/omp \
  --bind 127.0.0.1:7777 \
  --public-endpoint ws://127.0.0.1:7777/control \
  --tls-mode development-plaintext
```

In a second terminal, start the desktop client:

```sh
direnv exec . dx serve --desktop --package omp-app
```

In a third terminal, create a ten-minute pairing link:

```sh
direnv exec . cargo run -p ompd -- pair \
  --admin-socket /tmp/omp-remote-dev/ompd.sock \
  --name "Development desktop" \
  --expires 10m
```

Paste the printed `Native app` link into the client and select **Pair and connect**. This plaintext mode is deliberately restricted to a loopback listener; use one of the TLS deployment modes for remote access.

## Documentation

- [User guide](docs/user.md): pairing, agents, interactions, devices, and troubleshooting.
- [Setup and operations](docs/setup.md): development, daemon configuration, TLS, web deployment, and checks.
- [Architecture](docs/architecture.md): process boundaries, protocols, state, persistence, and security.

## Development checks

Run all repository checks through the flake environment:

```sh
direnv exec . nix flake check
```

Focused commands and formatting guidance are in [Setup and operations](docs/setup.md#development-workflow).

## Security

Treat pairing URLs and QR codes as short-lived credentials. Outside explicit local development, expose the control endpoint only as `wss://`. Native clients keep device credentials in the operating-system keyring; browser clients use browser local storage and therefore inherit the browser profile's security boundary. See [Security boundaries](docs/architecture.md#security-boundaries).
