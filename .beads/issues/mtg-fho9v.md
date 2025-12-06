---
title: Add RatZilla backend switching (dom/canvas/webgl2)
status: open
priority: 4
issue_type: task
created_at: 2025-12-06T16:06:50.103429998+00:00
updated_at: 2025-12-06T16:06:50.103429998+00:00
---

# Description

## Summary

Add ability to switch between RatZilla's three rendering backends:
- **DomBackend** (current default) - Most compatible, supports hyperlinks, slowest
- **CanvasBackend** - Good fallback with full Unicode support
- **WebGl2Backend** - Best performance, GPU-accelerated

## Implementation Notes

RatZilla provides three backends with different tradeoffs:
1. DomBackend: Renders cells as HTML elements
2. CanvasBackend: Uses Canvas 2D API
3. WebGl2Backend: GPU-accelerated via beamterm

### Current State

We currently use DomBackend exclusively (created in launch_fancy_tui).

### Proposed Changes

1. Add backend selector dropdown in web/fancy.html
2. Update launch_fancy_tui WASM function to accept backend type
3. Add WasmBackendType enum exported via wasm_bindgen
4. Consider using MultiBackendBuilder for fallback support

### References

- RatZilla backends: https://docs.rs/ratzilla/latest/ratzilla/backend/
- Parent issue: mtg-mf52n (TUI code sharing)
