# Introduction

DeepScry is a high-performance reimplementation of a Magic: The Gathering
rules engine in Rust, inspired by the Java [Forge](https://github.com/Card-Forge/forge)
project. It can run fully-automated games between AI players, drive interactive
games in a terminal or a web browser, and serve networked multiplayer matches.

This guide is organised into three parts.

- **Part I — User Guide.** How to drive the `mtg` command-line binary: the
  subcommands, running games, building reproducible bug reports, the deck-file
  format, scripted play, and the web server. Read this first if you want to
  *use* DeepScry.

- **Part II — Internal Architecture and Principles.** The engineering core:
  the network architecture, deterministic simulation, and the replicated
  state-machine model that keeps a server and its clients bit-for-bit in step.
  Read this if you want to understand *why* the engine is built the way it is.

- **Part III — Reference: The Scripting Languages.** Precise references for the
  three small languages DeepScry understands: the fixed-input controller
  language (for scripting a player's choices), the puzzle (`.pzl`) language
  (for setting up board states and self-checking assertions), and the
  card-script language (the card-definition format inherited from Java Forge).

## A note on accuracy

This guide cross-checks its claims against the actual source code wherever it
can. Some of the source documents it draws on had drifted from the code; where
that happened, this guide follows the code and **flags the discrepancy in an
admonition box** so you are never misled by a polished-looking but stale claim.
Look for blockquotes that begin with **Status:** or **Discrepancy:**.

## Building this guide

This book is compiled with [mdBook](https://rust-lang.github.io/mdBook/). From
the repository root:

```bash
make docs-guide        # or: mdbook build docs/guide
```

The compiled HTML is written into `web/guide/`, so it ships as part of the
deployed site. The Markdown sources live in `docs/guide/src/`; the compiled
output is **not** tracked in git (it is regenerated on each build, the same way
other generated content under `web/` is handled).
