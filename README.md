# mtg-forge-rs

An experiment in porting a subset of forge to Rust while adding tree search.

## Build Instructions

### Prerequisites

- **Rust nightly toolchain** (required for allocator_api and other unstable features)
  ```bash
  rustup default nightly
  # Or use rust-toolchain.toml (included in repo)
  ```

### Build Modes

The project uses Cargo feature flags to support different build configurations:

| Feature | Description | Use Case |
|---------|-------------|----------|
| `native` (default) | Full native build with CLI, TUI, async I/O, threading | Local development, testing |
| `network` | WebSocket client/server for multiplayer | Network play testing |
| `wasm` | Browser-compatible build (no threading) | Web deployment |
| `wasm-tui` | Browser TUI via RatZilla (DOM/WebGL2) | Web GUI |
| `verbose-logging` (default) | Detailed game event logging | Debugging |

### Common Build Commands

```bash
# Default build (native + verbose-logging)
cargo build --release

# With network multiplayer support
cargo build --release --features network

# Network-enabled binary (for test scripts)
cargo build --release --bin mtg --features network

# WASM build for browser
cargo build --release --target wasm32-unknown-unknown --features wasm-tui

# Performance-optimized build (no logging allocations)
cargo build --release --no-default-features --features native
```

### Binary Entry Points

- **`mtg`** - Main CLI binary (requires `native` feature)
  - `mtg tui <deck1> <deck2>` - Run a local game with TUI
  - `mtg tourney` - Run AI tournaments
  - `mtg profile` - Performance profiling
  - `mtg server` - Start network server (requires `network` feature)
  - `mtg connect` - Connect as network client (requires `network` feature)

### Testing

Tests use `tests/lib/test_helpers.sh` which ensures the mtg binary is built with network support:

```bash
# Run all tests via make
make validate

# Run specific test categories
cargo nextest run              # Unit tests
./tests/e2e_*.sh              # E2E shell tests

# Network mode testing (runs through server+clients)
MTG_NETWORK_MODE=1 ./tests/e2e_heuristic_v_random.sh
```

### Benchmarking

See `CLAUDE.md` for benchmarking conventions. Key points:
- Official benchmark entrypoint: `./scripts/run_benchmark.sh`
- Results recorded to `experiment_results/<CPU>/perf_history.csv`
- Use `--no-default-features --features native` for maximum performance

## Project Structure

```
mtg-engine/           # Core game engine library and CLI binary
  src/
    network/          # WebSocket client/server (feature-gated)
    game/             # Game state, rules, actions
    ai/               # Heuristic AI and evaluation
    tui/              # Terminal UI (native and WASM)
  decks/              # Sample deck files
  cardsfolder/        # Card definitions (from Java Forge)
web/                  # WASM web frontend
tests/                # E2E test scripts
  lib/test_helpers.sh # Shared test infrastructure
scripts/              # Build and benchmark scripts
```

## Related Documentation

- `CLAUDE.md` - Development guidelines, coding conventions, workflow
- `PROJECT_VISION.md` - Architecture goals and design principles
- `OPTIMIZATION.md` - Performance optimization strategies
- `docs/HOWTO_AGENTPLAY+REPRODUCERS.md` - Running AI games for testing
