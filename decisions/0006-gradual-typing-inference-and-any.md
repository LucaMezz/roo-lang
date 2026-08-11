# 0006. Refined gradual typing: infer-when-local-source, else dynamic, `any` overrides

**Status:** Accepted (supersedes an earlier, inconsistent statement of the
gradual typing rule)

## Context

The book originally stated a blanket rule: "no annotation means
dynamically typed." But the type-inference chapter separately, and
correctly, documented that `let count = 5;` gets *statically inferred* as
`int` and locked there — a direct contradiction. `let count = 5; count =
"oops";` can't simultaneously be a type error (inference) and allowed
(dynamic).

## Decision

The corrected rule: fig infers a static type from a local, unambiguous
source when one exists — today, that's only a `let` binding's own
initializer expression — and the binding is statically checked from then
on. Where there's no such local source (function parameters, struct/enum
fields, function return types — none of which have a single local
expression to infer from, since fig deliberately doesn't do
whole-program/call-site-driven inference), the position is dynamically
typed. `any` is a real, nameable builtin type (same status as `int` or
`String`, not a keyword) that explicitly overrides inference, forcing a
position to stay dynamic even where it would otherwise be inferable.

## Rationale

This is closer to how TypeScript and Luau actually behave than the
original blanket rule was — both infer `let`/`local` bindings from their
initializers by default, and both have an explicit `any` you can write to
opt out (Luau: `local x: any = 5`), rather than treating "no annotation"
as uniformly dynamic everywhere.

Also resolved as part of the same pass: an *omitted return type* is not a
special case that defaults to `()` (a tempting but incorrect analogy to
Rust, where that's exactly what happens) — a return type has no local
source either, so it follows the same dynamic-by-default rule as
parameters and fields. Write `-> ()` to actually guarantee nothing is
returned; write `-> any` to say "dynamic" explicitly instead of relying
on omission meaning the same thing.

fig's overall type system otherwise stays much closer to Rust than to
TypeScript: nominal (not structural) types, and inference scoped to `let`
initializers and generic call-site arguments only — no return-type
inference from a function body, no contextual/callback-parameter
inference.

See: `book/src/types/gradual-typing.md`, `book/src/types/inference.md`,
`book/src/types/overview.md`.
