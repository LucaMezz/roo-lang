# 0012. Tuple-struct fields get per-field `pub`, like named fields

**Status:** Accepted

## Context

Found while writing `examples/ecs.fig`: named-struct fields have
per-field `pub` (`field ::= "pub"? ident (":" type)?`), but the
tuple-struct grammar had no equivalent at all — no way to mark an
individual positional field public or private. Worked around by giving
`EntityId(int)` an associated `new` constructor instead of ever
constructing it with the bare tuple form from outside its module.

## Decision

Each positional field in a tuple struct has its own `pub`, exactly
mirroring named fields: `struct EntityId(pub int);`,
`struct Meters(float);` (private/opaque), and mixed cases are allowed the
same way Rust allows them.

## Rationale

Matches Rust exactly; no alternative was worth considering. The
constructor-based access pattern used in `examples/ecs.fig` was kept even
after this fix — not because the syntax doesn't work, but as a
deliberate newtype-opacity choice, now clearly a choice rather than a
forced workaround.

See: `book/src/data-types/structs.md` ("Tuple structs"),
`book/src/appendix/grammar.md`.
