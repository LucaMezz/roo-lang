# 0010. Number-to-`String` conversion is `ToString`, not `Into`/`From`

**Status:** Accepted

## Context

Found while writing `examples/save_game.fig`: there was no documented way
to turn an `int`/`float` into a `String` at all. `as` casting explicitly
stops at int/float/char/bool conversions; no format/interpolation
mechanism exists (macros are removed from fig entirely). Two shapes were
considered for filling this gap: a `ToString`-style trait (`fn
to_string(self) -> String`), or Rust's more general `Into<T>`/`From<T>`
conversion traits (`5.into()`).

## Decision

`ToString`, not `Into`/`From`. `int`/`float`/`bool`/`char` implement it
intrinsically — the same mechanism already established for the
arithmetic/comparison traits ([0007](0007-operator-trait-shape-rhs-output.md)):
no fig source behind the impl, guaranteed by the language itself. Kept as
one trait you implement directly, not split into `Display`+`ToString`
the way real Rust does it, for simplicity.

## Rationale

This is the actually-idiomatic Rust answer, not a simplification of it.
`Into`/`From` are for conversions *between comparable types* (`String::
from(a_str)`, numeric widening); `Display`/`ToString` exist specifically
for "produce human-readable text." Concretely, `5.into()` for a `String`
doesn't even compile in real Rust — there's no `impl From<i32> for
String` in the standard library. Reaching for `Into` here would have been
a mistake, not just a stylistic choice.

Unlike the arithmetic traits, this one isn't a hard bootstrapping problem
— an int-to-string conversion could, in principle, be written in fig
itself using `%`/`/`/`char` — so it's treated as intrinsic mainly for
consistency with how the other primitive trait impls already work, not
because fig source is fundamentally incapable of it. That's stated
explicitly rather than left implied.

See: `book/src/types/casting.md` ("Converting to `String`").
