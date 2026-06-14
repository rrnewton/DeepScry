# Overview of the `mtg` Binary

Everything in DeepScry is driven through a single command-line program, `mtg`.
It is a [clap](https://docs.rs/clap)-based multi-tool: you pick a subcommand,
then pass options to it.

```bash
cargo run --release --bin mtg -- <SUBCOMMAND> [OPTIONS]
# or, once built / deployed:
mtg <SUBCOMMAND> [OPTIONS]
```

The authoritative definition of every subcommand and flag lives in
`mtg-engine/src/main.rs`. When in doubt, run `mtg --help` or
`mtg <SUBCOMMAND> --help` — the help text is generated from that file and will
always match the binary you actually have.

## The subcommands

The complete list of subcommands, taken directly from the `Commands` enum in
`mtg-engine/src/main.rs`:

| Subcommand | Purpose |
| --- | --- |
| `tui` | Interactive / scripted gameplay. The main entry point for playing or simulating a single game. Also loads puzzles via `--start-state`. |
| `resume` | Resume a previously saved game from a snapshot file. |
| `profile` | Run many games back-to-back for profiling (flamegraph / heaptrack). |
| `tourney` | Run many games in parallel and collect win/loss statistics. |
| `stats` | Print statistics about the card database. |
| `deck-build` | Interactive fast deck-entry TUI for typing in paper decks. |
| `export-wasm` | Export the card database and selected decks for browser (WASM) builds. |
| `hash-asset` | Print the content-addressed hash of a file (a small scripting utility). |
| `hash-web-assets` | Hash the web-asset bundle (part of the content-addressed deploy pipeline). |
| `download` | Download card images from Scryfall. |
| `build-card-lookup` | Build the compact card→image lookup table used by the client. |
| `server` | Headless multiplayer WebSocket game server (no static files). |
| `server-web` | Full browser product: static web assets **and** the lobby on one port. |
| `connect` | Connect to a running multiplayer server as a client. |

> **Discrepancy (flagged):** Some older documents refer to an `mtg puzzle <file>`
> subcommand. **There is no `puzzle` subcommand in the binary.** Puzzles are
> loaded by passing a `.pzl` file to `tui` via `--start-state`:
> `mtg tui --start-state puzzles/bolt_test.pzl`. (Inside the `agentplay/`
> Python harness, `--puzzle` is a *script-level* convenience flag that ends up
> calling `tui --start-state` underneath; it is not a binary subcommand.)

## Feature flags

Some subcommands only exist when the binary is built with the right Cargo
feature:

- `server` and `server-web` require the `web-server` / `network` features.
- `connect` requires the `network` feature.

The standard release build used throughout this guide enables them:

```bash
cargo build --release --features network
```

The rest of Part I walks through the subcommands you will use most: running
games (`tui`, `resume`, `tourney`), the agent-play workflow for building
reproducers, the deck-file format, scripted play, card-lookup/asset building,
and the web server.
