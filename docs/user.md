# User guide

OMP Remote lets a paired device observe and control OMP agents running on the daemon host. The client can run as a desktop app or in a browser; both expose the same workspace.

## Before you start

You need:

1. A running `ompd` daemon with access to an OMP executable.
2. A client application.
3. A pairing link generated on the daemon host.

Operators can follow [Setup and operations](setup.md) to provide these pieces.

## Pair a device

On the daemon host, create a short-lived pairing grant:

```sh
direnv exec . cargo run -p ompd -- pair \
  --admin-socket /path/to/ompd.sock \
  --name "Alice's laptop" \
  --expires 10m
```

The command prints a terminal QR code, a `Native app` link, a `Browser` link, and the expiry time.

In the client:

1. Set a recognizable **Device name**.
2. Paste a pairing link or choose **Scan QR image** and select/capture an image containing the QR code.
3. Select **Pair and connect** before the link expires.

A pairing grant is one-time use. After a successful pairing, the daemon issues a device credential and the client saves the server as a profile. Later sessions can connect through **Saved servers** without a new pairing link, provided the credential has not been revoked.

> Pairing links and QR codes contain a secret in the URL fragment. The fragment is not sent in an HTTP request, but anyone who obtains the complete link before it expires can attempt to pair. Do not publish, log, or retain it unnecessarily.

### Credential storage

- **Native app:** the device credential is stored in the operating-system keyring. The non-secret server profile is stored in the platform configuration directory.
- **Browser:** the credential and server profile are stored in that browser profile's local storage. Clearing site data removes them.

## Work with agents

The **Agents** page lists daemon-managed workspaces and their authoritative lifecycle state.

### Launch or select an agent

- Enter a non-empty ID in **New agent ID** and select `+` to launch a new OMP process.
- Select an existing agent in the sidebar to inspect its run stream.

Agent IDs identify daemon workspaces. Launching an ID whose OMP process is already running is rejected rather than creating a duplicate.

### Send work

Use the composer at the bottom of the agent page:

| Action | Effect |
| --- | --- |
| **Send prompt** | Starts a prompt run for the selected agent. |
| **Steer now** | Sends guidance to the active turn. |
| **Queue follow-up** | Adds a message to be handled after the active work. |
| **Abort turn** | Aborts the current turn but leaves the agent process available. |
| **Stop** | Stops the supervised OMP process for that agent. |

The run view streams user and assistant messages, thinking text, tool starts/results, notices, and command output. Connection and agent status are shown separately, so a disconnected client is not evidence that the daemon stopped the agent.

The displayed transcript is a live client view backed by the daemon's bounded replay buffer, not an archival session browser.

### Resume another session

Expand **Resume another session**, enter a session JSONL path, and select **Resume**. The path is resolved by the OMP process on the daemon host, not by the remote device; it must therefore be accessible to the daemon's operating-system user.

## Answer agent interactions

When OMP requests a selection, confirmation, input value, or editor response, OMP Remote opens a dialog and attempts to acquire an exclusive interaction lease. Once acquired, the client can answer or cancel the request.

Only one client can own an interaction at a time. If another client already owns it, the dialog reports that the prompt is unavailable. The UI requests a two-minute lease and releases it after sending a response. Passive interaction notices can be dismissed without a lease.

## Manage paired devices

Open **Devices** to review credentials known to the daemon. Each card shows:

- Device name, platform, and ID.
- Enabled scopes: Observe, Prompt, Sessions, Stop, UI, and Admin.
- Creation, last-seen, and revocation timestamps.

Select **Revoke access** to invalidate another device. The current credential cannot revoke itself from the UI. Revocation is enforced by the daemon on subsequent authentication.

`ompd pair` currently grants all available scopes to the new device. The scope model is enforced per request even though the CLI does not yet expose scope selection.

## Disconnect and reconnect

Select **Disconnect** in the sidebar to stop the client runner and return to the connection screen. Disconnecting does not stop daemon agents. Choose the saved server profile to reconnect; the client retries transient connection failures with bounded exponential backoff and resumes its subscriptions when possible.

## Troubleshooting

### The pairing link expired

Run `ompd pair` again. Expired links cannot be renewed in the client.

### The link was already used

Pairing grants are consumed once. Generate a new link for each device or browser profile.

### The client cannot reach the daemon

- Confirm the daemon is running and its `--public-endpoint` is reachable from the client.
- Confirm the endpoint ends in `/control`.
- Use `wss://` for every non-development deployment.
- For `trusted-reverse-proxy`, confirm the proxy supports WebSocket upgrades and the daemon itself listens only on the configured loopback address.
- For local plaintext development, both `--bind` and the mode's local endpoint must be loopback; the public endpoint must use `ws://`.

### A saved server no longer connects

The device credential may have been revoked, the browser's site data or native keyring entry may be unavailable, or the server may now have a different identity. Create a fresh pairing link instead of weakening TLS verification.

### An interaction is read-only

Another connected client owns the interaction lease. Answer from the owning client or wait for its lease to expire.

### The daemon reports a stale admin socket

Verify that no daemon is using the socket, remove the stale socket file, and restart `ompd serve`. Never remove the socket of a running daemon.
