# Card Lookup and Web Assets

A handful of `mtg` subcommands exist to build the data the browser client needs:
the card database export, card images, and the compact card→image lookup table.
You only run these when preparing a web build or a deploy; they are not part of
playing a game.

## `stats` — inspect the card database

```bash
mtg stats
```

Prints statistics about the cards loaded from `cardsfolder/`. Useful as a quick
sanity check that the card database parses.

## `export-wasm` — package data for the browser

```bash
mtg export-wasm
```

Exports the card database and a selection of decks into the binary format the
WASM (browser) build loads. By default it includes the old-school and
booster-draft deck sets; glob patterns let you include more.

## `download` — fetch card images

```bash
mtg download --only-deck decks/old_school/01_rogue_rogerbrand.dck
```

Downloads card images from Scryfall. Options control the output directory, the
image sizes (`small`, `normal`), concurrency, and a politeness delay between
requests.

## `build-card-lookup` — the card→image table

```bash
mtg build-card-lookup
```

Builds the compact lookup table (`card-lookup.bin`) that maps each card to an
immutable Scryfall CDN image URL. It fetches Scryfall's `unique_artwork.json`
bulk dump once (cached locally), picks the oldest art per card identity, and
writes the table the client and the `download` command use to construct image
URLs. If Scryfall's URL format ever drifts, this command **hard-errors and keeps
the existing table** rather than writing something wrong. It is meant to run at
deploy time, not on every build.

## `hash-asset` / `hash-web-assets` — content addressing

These are part of the content-addressed asset pipeline used by the deploy. They
compute stable [BLAKE3](https://github.com/BLAKE3-team/BLAKE3) hashes of asset
files so the deployed site can cache aggressively and bust the cache precisely
when an asset's bytes change.

```bash
mtg hash-asset path/to/file        # print the 16-hex BLAKE3 hash of one file
```

The generated lookup table and image assets are **regenerated at deploy time**
and are not tracked in git.
