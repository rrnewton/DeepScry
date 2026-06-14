# The Card-Script Language

Every card in DeepScry is defined by a small text file in `cardsfolder/`, using a
key-value scripting language: `Name:`, `ManaCost:`, `Types:`, keyword lines
(`K:`), ability lines (`A:`, `T:`, `S:`), script variables (`SVar:`), and so on.

> **Important — this language is not ours to define.** The card-script DSL is
> **owned by the upstream Java [Forge](https://github.com/Card-Forge/forge)
> project**, which is the source of truth for its syntax and semantics. The
> `cardsfolder/` data itself comes from Forge. DeepScry *consumes and reproduces*
> this format so it can play the same cards; it does not get to invent or change
> the language. The reference in this guide documents the format **for reading
> and parsing**, not as a specification we control.

Because the format is structured, DeepScry parses it with proper tokenisation —
splitting on the `|` and `$` delimiters and querying the resulting fields — never
with ad-hoc substring matching. (Substring checks on structured data are
explicitly banned in the project conventions: `contains("add")` would match
"Madden", `contains("Damage")` would match "PreventDamage", and so on.) The
parsing infrastructure lives in the engine's ability-parser modules; see
`ai_docs/reference/ability_parsing_comparison.md` for the rationale.

The next page is the card-script specification itself, included from the
project's reference document.

> **Scope note.** The specification that follows describes the fields and ability
> shapes DeepScry recognises. Where DeepScry's coverage of a particular keyword
> or ability is incomplete relative to upstream Forge, that is tracked in the
> project's card-compatibility issues, not in this language reference — the
> *language* is upstream's; the *coverage* is DeepScry's ongoing work.
