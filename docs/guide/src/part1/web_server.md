# The Web Server and Deployment

DeepScry can serve networked multiplayer games. There are two server
subcommands and one client subcommand.

## `server` — headless lobby

```bash
mtg server --port 17771
```

A long-lived, headless WebSocket game server. It speaks the lobby protocol
defined in `mtg-engine/src/network/protocol.rs`: clients `Register` a unique
username, then `ListGames`, `CreateGame`, or `JoinGame`. The server hosts
multiple concurrent games. It serves **no** static files — it is the game engine
only.

Notable options: `--password` (require a join password), `--starting-life`,
`--seed` (deterministic games), `--max-memory-percent` (refuse new games above a
host-memory threshold; in-flight games are never killed), and `--network-debug`
(attach state hashes to every message and validate them after each choice — the
early-detection mechanism described in
[Network Architecture](../part2/network_architecture.md)).

## `server-web` — the full browser product

```bash
mtg server-web --bind 0.0.0.0:8080 --static-dir ./web
```

This is the single process that powers the deployed website. On one port it
serves both:

- **Static files** from `--static-dir` (default `./web`) at every path that is
  not the lobby WebSocket path: the lobby HTML, the WASM bundle, card-image
  data, JavaScript modules, and other assets.
- **A WebSocket lobby** at `--lobby-path` (default `/lobby`). The browser
  connects here for the whole flow (register, browse, create/join, set deck, set
  ready, play, reconnect, bug report). Behind the proxy, an embedded game server
  runs on a private loopback port that is never directly reachable from outside.

TLS is enabled when both `--tls-cert` and `--tls-key` (or the `MTG_TLS_CERT` /
`MTG_TLS_KEY` env vars) point at valid PEM files. When they are unset the server
speaks plain HTTP — the standard setup when a CDN such as Cloudflare terminates
TLS at its edge and forwards plaintext to the origin.

On `SIGTERM` or Ctrl-C the server stops accepting new connections, notifies open
clients with a fatal error message, and drains for up to 30 seconds.

## `connect` — join as a client

```bash
mtg connect decks/a.dck --server localhost:17771 --name alice
```

Connects to a running server with the given deck and joins the lobby.

## The web front end

The deployed front end lives under `web/`: a public landing page and lobby
(`web/index.html`), a WASM AI-vs-AI demo, and the in-game UIs. The landing page
collects a username, connects to the server over WebSocket, and launches into a
game page. It is intended to be self-explanatory in the browser, so this guide
does not walk through the on-screen controls.

## Deployment

Deployment is mechanised by `scripts/deploy-cloud.sh`, which has two phases:

- `deploy-cloud.sh config` — one-time per-VM bootstrap: installs the service
  unit, writes the environment file, opens the firewall port. Idempotent.
- `deploy-cloud.sh deploy` — run on every code change: rebuilds the WASM
  artefacts and the release `mtg` binary, rsyncs `web/`, `cardsfolder/`, and the
  binary to the VM, and restarts the service.

The deploy build uses the dedicated `release-deploy` Cargo profile (strip +
fat LTO + abort-on-panic) to produce a small binary suitable for rsync. All
site-specific values (user, host, ports, TLS paths, service name) come from a
gitignored local config file (`.deepscry-deploy.env`), CLI flags, or environment
variables — never hardcoded. See the deploy script and its `--help` for the
specifics; this guide intentionally keeps deployment at a high level.
