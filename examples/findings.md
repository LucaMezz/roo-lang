# Notes from writing worked examples

Four larger, involved programs (`ecs.roo`, `geometry.roo`, `save_game.roo`,
`dialogue.roo`) were written by hand against the book — no parser exists
yet, so "valid" here means "checked by hand against the grammar summary
and prose, sentence by sentence."

That process surfaced a number of real gaps and one grammar bug, all of
which turned into actual design decisions — those are recorded in
[`../decisions/`](../decisions/README.md), not here, since they're
project-wide, not specific to these four files. This document keeps only
what's actually specific to the examples themselves: what composed
cleanly without needing a fix, and the limits of what a hand-review like
this can tell you.

## What worked cleanly (worth knowing, not just the gaps)

- **`mut` on primitive-typed parameters** behaves exactly like `mut` on
  reference-typed ones (`fn use_item(mut health: int, ...)` in
  `save_game.roo`/`dialogue.roo`'s `heal`) — the rule turned out to be
  genuinely uniform, not just uniform-in-theory.
- **Fetching a component and mutating it** (get a reference-typed value
  out of storage via pattern match, then write through it) composes
  naturally, arguably more pleasantly than the equivalent Rust code, with
  no borrow-checker fighting.
- **Root-relative default path resolution** between sibling modules
  (`fs::read_to_string` called directly from inside `mod save`, no `use`,
  no `super::`) worked exactly as `modules.md` describes, first try.
- **Ambient modules generalize** beyond the book's one worked example
  (a struct+impl) — bare top-level ambient functions
  (`mod engine { pub fn delta_time() -> float; }`) and ambient functions
  returning `Result` both composed with the rest of the language with no
  special-casing needed.
- **`any` and the typed/untyped boundary** worked exactly as documented in
  both directions (`dialogue.roo`), with no surprises in either the
  "dynamic value into a typed function" or "typed value into a dynamic
  parameter" case.
- **Struct update syntax with a fully-qualified path**
  (`save::SaveData { level: data.level + 1, ..data }`, called from outside
  the `save` module) worked as expected.

## What this doesn't tell you

This was a hand-review, not a real parser/checker — it catches grammar
gaps and stated-rule contradictions, not runtime behavior, performance, or
whether the resulting programs are actually pleasant to write at scale.
