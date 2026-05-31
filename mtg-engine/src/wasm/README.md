# WASM Module

WebAssembly bindings for MTG Forge, enabling browser-based gameplay.

## Architecture

The WASM build exposes a JavaScript-friendly API through `wasm-bindgen`:

- Game state is managed in Rust and serialized to JSON for JS consumption
- Player controllers receive choices via callbacks from JavaScript
- Card and deck data is loaded from pre-serialized bincode files

## Module Structure

| File | Description |
|------|-------------|
| `mod.rs` | Core WASM exports: `WasmCardDatabase`, `WasmGame`, controller types |
| `human_controller.rs` | `WasmHumanController` for interactive human play in browser |
| `fancy_tui.rs` | Ratatui-based TUI rendered to browser via RatZilla (requires `wasm-tui` feature) |

## Exported API

### `WasmCardDatabase`
Card and deck data loaded from bincode files:
- `load_cards(data: Uint8Array)` - Load card definitions
- `load_decks(data: Uint8Array)` - Load deck lists
- `get_deck_names_json()` - List available decks
- `has_card(name)` / `has_deck(name)` - Check availability

### `WasmGame`
Game wrapper with JavaScript-friendly methods:
- `from_database(db, p1_deck, p2_deck, life, seed)` - Create game from loaded data
- `set_p1_controller(type)` / `set_p2_controller(type)` - Configure AI
- `run_ai_game(max_turns)` - Run to completion, returns JSON result
- `run_one_turn()` - Step one turn at a time
- `get_state_json()` - Current game state as JSON
- `get_logs_json()` - Game logs as JSON array

### `WasmControllerType`
Controller options: `Zero`, `Random`, `Heuristic`, `Human`

## JavaScript Usage Example

```javascript
import init, { WasmCardDatabase, WasmGame, version } from './mtg_engine.js';

async function main() {
    await init();
    console.log("MTG Forge version:", version());

    // Load card + deck data. Card data lives in per-set bins (mtg-464) and the
    // decks/tokens bins are content-addressed: resolve their hashed names from
    // the manifest (tokens+decks cache-skew fix) instead of a fixed URL.
    const setIndex  = await fetch('/data/sets/index.json').then(r => r.json());
    const decksData = await fetch(`/data/${setIndex.decks}`).then(r => r.arrayBuffer());

    const cardDb = new WasmCardDatabase();
    cardDb.load_decks(new Uint8Array(decksData));
    // Load only the per-set bins a deck needs, via setIndex.cards (see lib.rs docs).

    // Create a game with loaded decks
    const game = WasmGame.from_database(
        cardDb, "white_weenie_classic", "mono_black_control", 20, 12345
    );
    const result = game.run_ai_game(100);
    console.log("Game result:", JSON.parse(result));
}
```

## Data Generation

Use the CLI to generate bincode data files for WASM:

```bash
mtg export-wasm --output-dir ./web/data
```

This creates the per-set `data/sets/<YYYY>-<CODE>.<hash>.bin` bins,
`data/sets/index.json` (the resolver manifest), and the content-addressed
`data/tokens.<hash>.bin` + `data/decks.<hash>.bin` for loading in the browser.

## Limitations (vs. Native)

- No file system access (card/deck data must be provided from JS)
- No threading (single-threaded game loop)
- Token creation requires pre-loaded token definitions
- Network features work differently (WebSocket from browser context)

## Feature Flags

```toml
[features]
wasm-tui = ["ratzilla", "web-sys", ...]  # Browser TUI support
```

## Related Files

- `web/` directory: HTML/JS files for browser deployment
- `web/tui_game.html`: TUI gameplay in browser
- CI workflow: `.github/workflows/ci.yml` includes WASM build/test steps
