# MTG Forge WebAssembly Build

This directory contains a demo web interface for the MTG Forge game engine compiled to WebAssembly.

## Prerequisites

1. **Rust with wasm32 target**:
   ```bash
   rustup target add wasm32-unknown-unknown
   ```

2. **wasm-pack** (for building WASM packages):
   ```bash
   cargo install wasm-pack
   ```

## Building

From the project root directory:

```bash
# Build the WASM module
cd mtg-engine
wasm-pack build --target web --no-default-features --features wasm --out-dir ../web/pkg
```

This will create a `pkg/` directory containing:
- `mtg_forge_rs.js` - JavaScript bindings
- `mtg_forge_rs_bg.wasm` - WebAssembly binary
- `mtg_forge_rs.d.ts` - TypeScript definitions

## Running

You need a local web server to serve the files (browsers block loading WASM from `file://`):

```bash
# Option 1: Python (built-in)
cd web
python -m http.server 8080

# Option 2: Node.js serve
npx serve .

# Option 3: Any other static file server
```

Then open http://localhost:8080 in your browser.

## Features

The WASM demo provides:

- **Game creation**: Start a new game with configurable AI controllers
- **Turn-by-turn execution**: Run one turn at a time
- **Full game execution**: Run the game to completion
- **Game state display**: View life totals, turn info, and board state
- **Game logs**: View detailed game actions

## Limitations

The WASM build has some limitations compared to the native CLI:

1. **No card loading**: Cards/decks must be provided as data (not from files)
2. **No threading**: Game runs single-threaded
3. **Token creation**: Requires pre-loaded token definitions
4. **No TUI**: Only JSON-based state output (use web UI instead)

## API

The main JavaScript API:

```javascript
import init, { WasmGame, WasmControllerType, version } from './pkg/mtg_forge_rs.js';

// Initialize WASM module
await init();

// Create a new game
const game = new WasmGame("Player 1", "Player 2", 20);

// Set controller types
game.set_p1_controller(WasmControllerType.Heuristic);
game.set_p2_controller(WasmControllerType.Random);

// Set RNG seed for reproducibility
game.set_seed(BigInt(12345));

// Get game state as JSON
const state = JSON.parse(game.get_state_json());

// Run one turn (returns false if game ended)
const ongoing = game.run_one_turn();

// Run full game (returns result JSON)
const result = JSON.parse(game.run_ai_game(100)); // max 100 turns

// Get logs
const logs = JSON.parse(game.get_logs_json());
```

## Development

To modify the WASM bindings, edit:
- `mtg-engine/src/wasm/mod.rs` - Rust WASM bindings
- `web/index.html` - Demo web interface

After changes, rebuild with `wasm-pack build ...` as shown above.
