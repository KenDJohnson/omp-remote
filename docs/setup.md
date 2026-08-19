# Setup and operations

This guide covers source-based development and deployment of the current workspace. The repository does not define an installer or service manager; production process supervision and reverse-proxy configuration remain operator responsibilities.

## Prerequisites

### Supported development hosts

The Nix flake defines development shells for:

- `aarch64-darwin`
- `x86_64-darwin`
- `aarch64-linux`
- `x86_64-linux`

Install:

1. [Nix](https://nixos.org/download/), with flakes enabled.
2. [direnv](https://direnv.net/) and its shell hook.
3. An `omp` executable that supports `--mode rpc`.

The flake supplies Rust, rust-analyzer, Dioxus CLI, WebAssembly tooling, Node.js, cargo-nextest, audit/deny tools, and repository formatters. Do not install project tools ad hoc.

### Enter the environment

```sh
direnv allow
direnv exec . cargo build --workspace
```

`.envrc` uses `use flake`; `flake.nix` is the source of truth for development and test dependencies.

## Local development setup

The safest local topology uses a loopback-only plaintext daemon and the native desktop client.

### 1. Start the daemon

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

The daemon creates parent directories for the database and admin socket. The local admin socket is owner-only (`0600`) and is removed during a normal shutdown.

`--omp` defaults to `omp`; omit the option if the correct executable is already on `PATH`.

### 2. Start a client

Desktop:

```sh
direnv exec . dx serve --desktop --package omp-app
```

Browser development:

```sh
direnv exec . dx serve --web --package omp-app --port 8080
```

In the split-port browser development topology, paste the printed pairing payload into the app. The generated `Browser` URL assumes a same-origin production gateway and therefore points at the control endpoint's origin, not the Dioxus development server.

### 3. Create a pairing grant

With the daemon still running:

```sh
direnv exec . cargo run -p ompd -- pair \
  --admin-socket /tmp/omp-remote-dev/ompd.sock \
  --name "Development device" \
  --expires 10m
```

Accepted duration suffixes are `ms`, `s`, `m`, and `h`; the value must be a positive integer. Paste the `Native app` link into either development client.

## Daemon configuration

`ompd serve` requires an explicit database, local admin socket, bind address, public control endpoint, and TLS mode.

| CLI option | Environment variable | Meaning |
| --- | --- | --- |
| `--database` | `OMPD_DATABASE` | SQLite state database |
| `--admin-socket` | `OMPD_ADMIN_SOCKET` | Owner-only Unix administration socket |
| `--omp` | `OMPD_OMP` | OMP executable; defaults to `omp` |
| `--bind` | `OMPD_BIND` | TCP listener address |
| `--public-endpoint` | `OMPD_PUBLIC_ENDPOINT` | Client-visible WebSocket URL ending in `/control` |
| `--tls-mode` | `OMPD_TLS_MODE` | Transport mode listed below |
| `--tls-certificate` | `OMPD_TLS_CERT` | PEM certificate chain for direct TLS modes |
| `--tls-private-key` | `OMPD_TLS_KEY` | PEM private key for direct TLS modes |

The `pair` subcommand accepts `OMPD_ADMIN_SOCKET`; `--name` and `--expires` are command-line options.

### TLS modes

| Mode | Listener constraint | Public endpoint | Client identity check | Certificate options |
| --- | --- | --- | --- | --- |
| `certificate` | Any configured bind address | `wss://…/control` | Public trust store | Required |
| `pinned-self-signed` | Any configured bind address | `wss://…/control` | SHA-256 fingerprint from the pairing bundle | Required |
| `trusted-reverse-proxy` | Exact loopback bind address | `wss://…/control` | Public trust at the proxy | Rejected |
| `development-plaintext` | Exact loopback bind address | `ws://…/control` | Explicit insecure-development marker | Rejected |

Direct TLS is TLS 1.3 with HTTP/1.1 ALPN. The certificate file may contain a PEM chain; the private-key file must contain a supported PEM private key.

Example direct certificate deployment:

```sh
direnv exec . cargo run --release -p ompd -- serve \
  --database /var/lib/omp-remote/ompd.sqlite3 \
  --admin-socket /var/run/omp-remote/ompd.sock \
  --omp /opt/omp/bin/omp \
  --bind 0.0.0.0:7443 \
  --public-endpoint wss://remote.example.com:7443/control \
  --tls-mode certificate \
  --tls-certificate /etc/omp-remote/fullchain.pem \
  --tls-private-key /etc/omp-remote/private-key.pem
```

Ensure the service user owns the database and admin-socket directories. Keep the admin socket local; it creates all-scope pairing grants.

## Reverse proxy and browser deployment

Build the web client:

```sh
direnv exec . dx build --web --release --package omp-app
```

A browser deployment needs one HTTPS origin that:

1. Serves the built Dioxus application and falls back to its entry point for `/pair`.
2. Proxies `/control` as a WebSocket to an `ompd` loopback listener.
3. Preserves WebSocket upgrade semantics and does not expose the daemon listener directly.

Run the daemon with a matching endpoint, for example:

```sh
direnv exec . cargo run --release -p ompd -- serve \
  --database /var/lib/omp-remote/ompd.sqlite3 \
  --admin-socket /var/run/omp-remote/ompd.sock \
  --omp /opt/omp/bin/omp \
  --bind 127.0.0.1:7777 \
  --public-endpoint wss://remote.example.com/control \
  --tls-mode trusted-reverse-proxy
```

`ompd pair` derives the browser URL by replacing `wss://` with `https://`, removing the final `/control`, and appending `/pair`. The route and static assets must therefore be available on the same public origin as the control endpoint.

## Native credential-store requirements

Native clients use the platform's secure credential facility:

- macOS: Keychain
- iOS: protected keychain storage
- Android: native keyring storage
- Windows: native credential storage
- Other Unix systems: Secret Service over D-Bus

On Linux, ensure a Secret Service provider and user D-Bus session are available before pairing.

## Development workflow

Focused checks while iterating:

```sh
direnv exec . cargo nextest run -p omp-control-protocol
direnv exec . cargo clippy --workspace --all-targets --all-features -- --deny warnings
direnv exec . cargo fmt --all -- --check
```

Build both app targets explicitly when changing shared UI or client code:

```sh
direnv exec . dx build --desktop --package omp-app
direnv exec . dx build --web --package omp-app
```

Format Nix and TOML with tools from the dev shell:

```sh
direnv exec . alejandra .
direnv exec . taplo format
```

Before committing workspace changes, run the complete flake checks:

```sh
direnv exec . nix flake check
```

The flake checks formatting, clippy, Rust documentation, native tests, WebAssembly builds/tests, dependency policy, and RustSec advisories.
