---
title: 'Phase 2: Add TUI screenshot capture to FancyFixed controller'
status: open
priority: 3
issue_type: task
created_at: 2025-11-30T19:44:01.801505163+00:00
updated_at: 2025-11-30T19:44:01.801505163+00:00
---

# Description

## Phase 2: Add TUI Screenshot Capture to FancyFixed Controller

## Context

Phase 1 is complete (2025-11-30_#218): We added a `--p1=fancy-fixed` controller that accepts scripted inputs via `--p1-fixed-inputs`. Currently it's a thin wrapper around RichInputController - it runs the script but doesn't render TUI or capture screenshots.

**Phase 1 Accomplishments:**
- Added `FancyFixed` variant to `ControllerType` enum (main.rs + snapshot.rs)
- Created `FancyFixedController` struct (fancy_fixed_controller.rs)
- Wired up controller creation in main.rs (both initial game and resume modes)
- All trait methods delegate to `RichInputController`
- Compiles and runs successfully

## Goal for Phase 2

Enable automated TUI screenshot capture at each choice point, allowing the agent to:
1. See exactly what the TUI displays during gameplay
2. Debug TUI rendering issues without interactive terminal
3. Have a visual record of game state for analysis

## Technical Approach

### Option A: ratatui TestBackend (Recommended)

ratatui provides `TestBackend` which renders to an in-memory buffer that can be saved as text.

**Implementation:**
1. Add `TestBackend` to `FancyFixedController`
2. Before each choice, render the full TUI state to the backend
3. Extract buffer as string and save to file
4. Continue with scripted choice from RichInputController

**Code locations:**
- `mtg-engine/src/game/fancy_fixed_controller.rs` - Add rendering
- `mtg-engine/src/game/fancy_tui_controller.rs` - Reuse draw methods

**Example:**
```rust
use ratatui::backend::TestBackend;
use ratatui::Terminal;

// In FancyFixedController
struct FancyFixedController {
    fixed: RichInputController,
    screenshot_dir: Option<PathBuf>,
    screenshot_count: u32,
    // Add these:
    terminal: Terminal<TestBackend>,
    tui_state: FancyTuiState, // Internal state from FancyTuiController
}

impl FancyFixedController {
    fn new(...) -> Result<Self, MtgError> {
        let backend = TestBackend::new(120, 40); // 120x40 terminal
        let terminal = Terminal::new(backend)?;
        // Initialize TUI state
        ...
    }

    fn capture_screenshot(&mut self, context: &str) -> io::Result<()> {
        if let Some(ref dir) = self.screenshot_dir {
            self.screenshot_count += 1;
            let filename = format!("screenshot_{:04}_{}.txt", self.screenshot_count, context);
            let path = dir.join(filename);
            
            // Get buffer from TestBackend
            let buffer = self.terminal.backend().buffer();
            let content = buffer.content();
            
            // Convert buffer to string
            let mut output = String::new();
            for (i, cell) in content.iter().enumerate() {
                if i > 0 && i % 120 == 0 {
                    output.push('\n');
                }
                output.push_str(&cell.symbol);
            }
            
            std::fs::write(&path, output)?;
            log::info!("Saved screenshot: {}", path.display());
        }
        Ok(())
    }
}
```

### Option B: Extract rendering logic from FancyTuiController

Alternative: Make FancyTuiController's rendering methods public and reuse them.

**Challenges:**
- FancyTuiController is tightly coupled to CrosstermBackend
- Would need to refactor to support generic backend
- More invasive changes to existing code

**Recommendation:** Start with Option A (TestBackend) as it's less invasive.

## Code Files Involved

### Primary file to modify:
- `mtg-engine/src/game/fancy_fixed_controller.rs` (currently ~160 lines)

### Files to reference:
- `mtg-engine/src/game/fancy_tui_controller.rs` (3233 lines)
  - Study rendering methods: `draw_*`, `render_*`
  - Understand TUI state management
  - Lines 271-2736: All the drawing logic we want to reuse

### Dependencies to add:
- Already have `ratatui` in Cargo.toml (for FancyTuiController)
- `TestBackend` is part of ratatui, no new deps needed

## Screenshot Format

**Filename:** `screenshot_{NNNN}_{context}.txt`
- NNNN: 4-digit counter (0001, 0002, ...)
- context: spell_ability, attackers, blockers, target, library

**Location:** Same directory as `game.snapshot` (from `--snapshot-output`)
- For agentplay: `agentplay/current.game/screenshot_*.txt`

**Content:** Plain text ASCII art matching TUI display
- 120 columns x 40 rows (configurable)
- All TUI panes: hand, battlefield, stack, combat, card details
- Exact visual match to what Fancy TUI shows

## Testing Plan

Once implemented, test with:

```bash
## Test with Peter Porker bug reproducer
./agentplay/start_game.sh \
    decks/peter_porker_test.dck \
    decks/peter_porker_test.dck \
    --p1-draw="Forest;Spider-Ham, Peter Porker;Forest;Forest;Forest;Forest;Forest"

## Continue with FancyFixed (not yet implemented - will work after Phase 2)
RUST_LOG=tui=debug ./agentplay/continue_game.sh "play forest"
RUST_LOG=tui=debug ./agentplay/continue_game.sh "play forest;cast spider-ham"

## Verify screenshots created
ls -la agentplay/current.game/screenshot_*.txt

## Examine screenshot content
cat agentplay/current.game/screenshot_0001_spell_ability.txt
```

## Alternative: Simpler Text Dump (If TUI rendering too complex)

If TestBackend proves difficult, fallback to simpler approach:

```rust
fn capture_game_state_text(&self, view: &GameStateView, context: &str) -> String {
    let mut output = String::new();
    
    output.push_str("=== BATTLEFIELD ===\n");
    for card_id in view.battlefield() {
        if let Some(card) = view.get_card(*card_id) {
            output.push_str(&format!("{} ({})\n", card.name, card_id.as_u32()));
        }
    }
    
    output.push_str("\n=== HAND ===\n");
    for card_id in view.hand() {
        if let Some(card) = view.get_card(*card_id) {
            output.push_str(&format!("{}\n", card.name));
        }
    }
    
    output.push_str(&format!("\n=== CONTEXT: {} ===\n", context));
    output
}
```

## Success Criteria

✅ `--p1=fancy-fixed` creates screenshot files
✅ Screenshots are readable text files
✅ Screenshots show battlefield, hand, stack at each choice
✅ Agent (Claude Code) can read and analyze screenshots
✅ Debugging Peter Porker bug becomes possible without interactive terminal

## Related Issues

- mtg-4c13dc: Peter Porker TUI rendering bug (the motivating use case)
- This issue tracks Phase 2 implementation

---

When implementing, start with Option A (TestBackend) and the simpler methods first (choose_spell_ability_to_play). Once one method works, the pattern applies to all others.
