# 0007. Operator traits take Rhs (defaulting to Self) and an Output type

**Status:** Accepted (supersedes the original fixed-shape operator traits)

## Context

Found while writing `examples/geometry.fig`: the originally-documented
operator trait shape was fixed — `trait Add { fn add(self, other: Self)
-> Self; }`. That works for `Vector2 + Vector2`, but not for `Vector2 *
float` (scalar multiplication) — one of the most common operator
overloads in exactly the domain (game/engine math) fig targets. There was
no way for an operator impl to take a right-hand-side type other than
`Self`, or return something other than `Self`.

Two shapes were considered: a generic `Rhs` type parameter with no
default (simpler grammar, but every impl — even the common `Self op
Self` case — has to spell out `Rhs` explicitly), or `Rhs` defaulting to
`Self` (matches Rust exactly, keeps the common case as terse as the old
fixed shape, but requires fig to support default generic type
parameters, which it didn't yet).

## Decision

Operator traits take the real Rust shape:

```fig
trait Add<Rhs = Self> {
    type Output;
    fn add(self, rhs: Rhs) -> Self::Output;
}
```

This required adding **default generic type parameters** (`<T = Default>`)
to fig's generics system generally, not just to operator traits.

## Rationale

Rhs-with-a-default was chosen over Rhs-with-no-default specifically so
`impl Add for Vector2` (the common, same-type case) stays exactly as
terse as it was under the old fixed shape — the whole point was not to
make the 90% case pay for supporting the 10% case. Default type
parameters are documented as a general generics feature (not
special-cased to operators) since there's no reason to restrict a useful,
low-complexity feature to one use site.

A consequence that had to be fixed alongside this: a bare `T: Add` bound
on a generic function no longer guarantees the result is the same type as
the operands (only that *some* `Output` exists) — `fn sum<T: Add>(a: T, b:
T) -> T::Output { a + b }` is the correct generic signature, not `-> T`.

Not covered by this fix: the book still has no illustrative shape for the
equality/ordering traits, so a custom `==`/`<` overload still has no
documented method name to define.

See: `book/src/abstraction/operator-overloading.md`,
`book/src/abstraction/generics.md` ("Default type parameters"),
`examples/geometry.fig`.
