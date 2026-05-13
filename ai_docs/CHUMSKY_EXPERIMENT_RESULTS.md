# Chumsky Parser Experiment Results

## Overview

This document summarizes the experimental chumsky-based parser implementation for PZL files, created to evaluate error message quality and compare approaches with the manual parser.

## Success Rate

| Parser | Files Parsed | Success Rate |
|--------|-------------|--------------|
| Manual Parser (format.rs) | 351/351 | 100% |
| Chumsky Parser (parser_chumsky.rs) | 349/351 | 99.4% |

### Failures

The chumsky parser fails on 2 files due to multi-line values in card specifications:
- `forge_tutorial02.pzl` - Card specification spans multiple lines
- `forge_tutorial03.pzl` - Card specification spans multiple lines

Example problematic pattern:
```
humanbattlefield=... Swamp
|Set:DTK
```

The value parser uses `none_of("\r\n")` which cannot handle multi-line values. The manual parser handles this edge case through more complex line continuation logic.

## Lines of Code

| Parser | Implementation LOC | Total LOC (with tests) |
|--------|-------------------|------------------------|
| Manual | ~153 | 257 |
| Chumsky | ~191 | 360 |

The chumsky parser is slightly more verbose in implementation but has more comprehensive error testing.

## Error Message Quality

### Missing Required Field (Name)

**Chumsky:**
```
Simple {
  span: 0..0,
  reason: Custom("Missing required field 'Name' in [metadata] section"),
  expected: {},
  found: None,
  label: None
}
```

**Manual Parser:**
No explicit check - would use default value from PuzzleMetadata::default()

**Winner:** Chumsky - Clear, descriptive error message

### Missing Section ([metadata])

**Chumsky:**
```
Simple {
  span: 16..16,
  reason: Unexpected,
  expected: {Some('#'), Some('[')},
  found: None,
  label: None
}

Custom("Missing [metadata] section")
```

**Manual Parser:**
Would fail during metadata parsing with less context about what's missing

**Winner:** Chumsky - Shows both parse-level and semantic-level errors

### Invalid Syntax

For a file with syntax errors, chumsky provides:
- Exact character position (span)
- What was expected
- What was found

The manual parser would typically fail with a generic parse error or panic.

## Advantages of Chumsky Approach

1. **Better Error Messages**: Clear, structured errors with position information
2. **Declarative Grammar**: Parser structure directly reflects the grammar
3. **Type Safety**: Parser combinator types catch errors at compile time
4. **Composability**: Easy to build complex parsers from simple ones

## Advantages of Manual Approach

1. **Edge Case Handling**: Can handle multi-line values and other quirks
2. **Slightly More Concise**: ~20% fewer lines of code
3. **More Forgiving**: Uses defaults liberally, always succeeds
4. **Full Control**: Can implement custom logic for special cases

## Performance

Benchmark results on a 1165 byte puzzle file (PC_062315.pzl):

| Parser | Parse Time | Throughput | Relative Speed |
|--------|-----------|------------|----------------|
| Manual | ~1.03 µs | 1.05 GiB/s | 1.0x (baseline) |
| Chumsky | ~40.8 µs | 27.2 MiB/s | 0.025x (40x slower) |

**Key Finding:** The manual parser is approximately **40x faster** than the chumsky parser.

### Performance Analysis

The significant performance difference is due to:

1. **String Operations vs Parser Combinators**: Manual parser uses direct string splitting and pattern matching, while chumsky builds a parser combinator tree with abstraction overhead

2. **Allocation Patterns**: Chumsky allocates intermediate parser results and error structures even for successful parses

3. **Type System Overhead**: Parser combinator composition requires trait dispatch and generic instantiation

For a 1KB file:
- Manual: ~1 microsecond (negligible)
- Chumsky: ~40 microseconds (still fast, but noticeable at scale)

Even at 40µs per file, chumsky could parse all 351 files in ~14ms, which is acceptable for non-critical path operations.

## Conclusion

The chumsky parser achieves 99.4% success rate while providing significantly better error messages. For a user-facing tool where error clarity matters, chumsky would be the better choice despite slightly more code.

For the current use case (internal tooling with controlled input), the manual parser's 100% success rate and flexibility with edge cases makes it more practical.

The experiment successfully demonstrates that:
1. PZL format can be formally described with parser combinators
2. Error messages can be dramatically improved with minimal effort
3. The grammar is simple enough that either approach works well
