# 0008. `mut` binds to individual identifiers inside a pattern

**Status:** Accepted (supersedes separate `mut` slots on `let`/parameters)

## Context

Found while writing `examples/ecs.fig`: `if let Option::Some(mut p) = ...`
had no basis in the documented grammar — the pattern grammar had no `mut`
anywhere, unlike Rust, which allows `mut` on any individual binding within
a pattern. This blocked a common, ordinary thing to want: pull a
reference-typed value out of storage via a match/if-let, then mutate it.
The only workaround was binding immutably and immediately shadowing
(`let mut p = p;`).

## Decision

`mut` can precede any identifier binding inside a pattern — `Some(mut
x)`, `(mut a, b)`, `Point { mut x, y }` — applying only to that one
binding, everywhere a pattern can appear (`let`, `if let`, `while let`,
`match` arms, function parameters, `for` loop variables).

As a direct consequence, the special-cased `"mut"?` slot that used to sit
on `let_stmt` and on function `param` was removed from the grammar
entirely. `let mut x = 5;` is now understood as `let` applied to the
pattern `mut x` — a single-identifier pattern with `mut` on it — the same
mechanism as everywhere else, not a second mechanism that happens to look
similar.

## Rationale

This is the same trap Rust's own grammar avoids the same way: real Rust
has no separate `mut` slot on `let` either — `let mut x = 5;` is `let`
applied to the pattern `mut x`, full stop. Unifying fig's grammar the same
way isn't just a simplification, it's *more* faithful to how Rust actually
works, not less.

See: `book/src/data-types/pattern-matching.md` ("Mutable bindings"),
`book/src/bindings/variables.md`, `book/src/appendix/grammar.md`.
