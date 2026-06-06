---
name: code-elegance-and-correctness
description: >
  Focus on improving codebase quality, cleanliness, and correctness. Replaces
  conditional/special-case hacks with abstract structures derived from first-principles
  MTG rules (stack, effects, zones). Follows CLAUDE.md conventions (strong types, DRY,
  modular design, keeping functions/files focused).
---

# Code Elegance and Correctness

Improving the quality of this codebase is a joint focus on ELEGANCE and CORRECTNESS, because the two are deeply related.

CLAUDE.md in this project already emphasizes several rules of thumb and coding conventions (strong types, files and functions not too long, modularity, DRY).

## Core Principles

- **Eliminate Conditional Hacks**: Any conditional hacks (e.g., `if (special_circumstances) { do_hack; }`) are a code smell that must be eliminated.
- **Model Rules Explicitly**: By reading the full MTG rules, understand from first principles how the engine should execute (stack, effects, zones, etc.). Refactor code to follow these abstract structures. If rules are modeled explicitly, complex interactions should emerge naturally rather than requiring special-case logic.
- **Identify and Address Smells**: Read code to identify problematic patterns, or take tasks from the backlog.
- **Add Tests**: When identifying smells or refactoring, write tests—preferring end-to-end (e2e) game tests—to exercise the logic.
- **Clean Code Engineering**: Look for opportunities to improve modularity and generality. Apply general-purpose software engineering guidelines, such as:
  - Fixing functions with too many arguments.
  - Factoring out too-large or too-indented/nested functions.
