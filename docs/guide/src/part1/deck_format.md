# Deck Files (`.dck`)

DeepScry reads decks in Forge's `.dck` format — a plain-text, INI-style format
that is compatible with Java Forge deck files. This chapter is a practical
summary; the full specification is in `docs/DCK_FORMAT.md`, and the parser lives
in `mtg-engine/src/loader/deck.rs`.

## File structure

A `.dck` file has up to three sections:

```ini
[metadata]
Name=Lightning Bolt Burn
Description=Classic red burn deck

[Main]
40 Lightning Bolt
20 Mountain

[Sideboard]
15 Shock
```

### `[metadata]`

Key-value pairs. `Name` is required; `Description` is optional. Unknown metadata
keys are silently ignored.

### `[Main]` and `[Sideboard]`

Each line is a quantity followed by a card name:

```text
<quantity> <card name>[|<set code>][|<art index>]
```

- `<quantity>` — number of copies (typically 1–255).
- `<card name>` — the full card name as printed.
- `<set code>` — optional three-letter set code (e.g. `LEA`, `M10`).
- `<art index>` — optional 1-based art-variant index.

Examples:

```text
4 Lightning Bolt
3 Lightning Bolt|M10
1 Lightning Bolt|M10|2
20 Mountain
```

The `[Sideboard]` section uses the same line format as `[Main]`.

## How card names are matched

When the loader maps a deck line to a card file in `cardsfolder/`, it normalises
the name: lowercase it, replace spaces and hyphens with underscores, and strip
apostrophes, commas, colons, exclamation marks, and question marks. So:

| Card name | Resolved file |
| --- | --- |
| `All Hallow's Eve` | `all_hallows_eve.txt` |
| `Nevinyrral's Disk` | `nevinyrrals_disk.txt` |
| `Jace, the Mind Sculptor` | `jace_the_mind_sculptor.txt` |

Matching is case-insensitive.

## Current implementation status

Taken from `docs/DCK_FORMAT.md`:

**Supported:** the `[metadata]`, `[Main]`, and `[Sideboard]` sections; quantity
parsing; card-name normalisation; comments (`#`) and blank lines.

**Parsed but not yet acted on:** set codes and art indices are *parsed* but
currently ignored during loading — the loader resolves a card by its normalised
name only. Additional metadata fields beyond `Name`/`Description` are ignored.

> **Note:** the `.dck` format is shared with Java Forge by design, so DeepScry
> can read the historical Forge deck corpus directly. Example decks live under
> `decks/` (see `decks/old_school/` for complex historical lists).
