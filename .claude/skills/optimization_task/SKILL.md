---
name: optimization-task
description: >
  Guidelines for selecting, profiling, implementing, and validating performance
  optimizations. Focuses on zero-copy patterns and minimizing allocation rates,
  using OPTIMIZATION.md as the primary technical reference.
---

# Optimization Task

This skill outlines the process for picking, implementing, and validating performance optimization tasks. Technical details must be referred to [OPTIMIZATION.md](file:///home/newton/work/dev-deepscry/deepscry/OPTIMIZATION.md).

## Process Workflow

1. **Verify Clean Base**: Ensure your working directory is clean as described in `CLAUDE.md`.
2. **Review Context**:
   - Outstanding performance tracking issues (e.g., run `bd show mtg-2` or check minibeads).
   - [OPTIMIZATION.md](file:///home/newton/work/dev-deepscry/deepscry/OPTIMIZATION.md) for details on zero-copy patterns, memory profiling, and metrics.
   - `CLAUDE.md` and `PROJECT_VISION.md`.
3. **Select Task**: Choose a high-priority optimization task from the backlog or identified through profiling.
4. **Profile & Implement**: Use profiling tools (e.g., Callgrind, Heaptrack) to find bottlenecks and apply zero-copy patterns.
5. **Verify Correctness & Performance**:
   - Run `make validate` to guarantee no gameplay regressions.
   - Run `make bench` (or specific cargo bench commands) to measure the impact of the changes.
6. **Strict Commit Rule**:
   - **DO NOT commit** an optimization change to `integration` / `main` unless it **measurably improves** at least one key metric (e.g., actions-per-second or byte-allocations-per-turn).
   - If the optimization failed but might be useful to preserve, commit it to a feature branch named `failed-optimization-XY`.
   - If stuck, write the problem to `error.txt` before exiting.

---

## Technical Checklist (from OPTIMIZATION.md)

Refer to [OPTIMIZATION.md](file:///home/newton/work/dev-deepscry/deepscry/OPTIMIZATION.md) for the exact implementation details of each of these principles:

### Zero-Copy Patterns & Allocation Principles
- [ ] **Avoid Unnecessary `clone()`**: Use references, lifetimes, and `iter().cloned()` where appropriate.
- [ ] **Avoid Unnecessary `collect()`**: Return `impl Iterator` to avoid heap-allocating intermediate collections.
- [ ] **Chain Iterator Operations**: Combine map/filter steps into a single traversal.
- [ ] **Use Slices Instead of Owned Types**: Prefer `&str` and `&[T]` over `&String` and `&Vec<T>`.
- [ ] **Implement `size_hint()`**: Help collections pre-allocate during `collect()` or `extend()`.
- [ ] **Arena Allocation**: Utilize bumping or arena allocations for short-lived, turn/phase-bounded structures.
- [ ] **Object Pools**: Reuse objects for frequent allocations (e.g. combat buffers or tokens).
- [ ] **Use `SmallVec` and `SmallMap`**: Keep small inline allocations on the stack instead of the heap.
- [ ] **Prefer Unboxed Enums**: Avoid boxed trait object vectors to prevent pointer-chasing and allocation fragmentation.
- [ ] **Use `Cow`**: Defer cloning until mutation is explicitly required.

### Profiling and Measurement Sections
- [ ] **Tracking Metric**: Focus on `robots_mirror/mem_logging_rewind_play_again` (Actions/sec and Bytes/game).
- [ ] **CPU Profiling**: Run `make callgrindprofile` to generate instruction-count profiles.
- [ ] **Memory Profiling**: Use Heaptrack to find allocation counts and sizes.
