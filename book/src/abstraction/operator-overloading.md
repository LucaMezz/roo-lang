# Operator Overloading

Operators on custom types are implemented the same way Rust does it: by
implementing a specific, operator-associated trait for your type. Writing
`a + b` is sugar for calling that trait's method — for `+`, conceptually an
`Add` trait shaped like:

```fig
trait Add {
    fn add(self, other: Self) -> Self;
}
```

so that implementing it for a custom type makes `+` work on that type:

```fig
struct Point { x: float, y: float }

impl Add for Point {
    fn add(self, other: Point) -> Point {
        Point { x: self.x + other.x, y: self.y + other.y }
    }
}

let sum = Point { x: 1.0, y: 2.0 } + Point { x: 3.0, y: 4.0 };
```

`Self` (capital `S`) inside a trait or `impl` block refers to "the type this
block is implementing the trait for" — here, `Point`.

## What can be overloaded

The same set of operators Rust allows overloading for: the binary
arithmetic operators (`+`, `-`, `*`, `/`, `%`), the bitwise operators (`&`,
`|`, `^`, `<<`, `>>`), unary negation (`-`) and unary bitwise/logical
`not` (`!`), indexing (`[]`), and the comparison operators (`==`/`!=` via an
equality trait, `<`/`<=`/`>`/`>=` via an ordering trait). Each corresponds to
one operator-specific trait, the same way Rust's `std::ops` module defines
one trait per overloadable operator.

The exact names and method signatures of these traits (`Add`, `Sub`,
`PartialEq`, `PartialOrd`, `Index`, and so on) belong to the standard
library, which isn't finalized yet — see [Introduction](../introduction.md).
This page describes the *mechanism* (operator syntax desugars to a trait
method call, and implementing the trait for your type makes the operator
work), which is a language-level guarantee independent of what the traits
end up being named.

## Primitive types implement these traits intrinsically

`int`, `float`, `bool`, and `char` already support the relevant operators —
`1 + 1` doesn't require anyone to have written an `Add` impl. But they
still need to *nominally* implement these traits, not just have their
operators special-cased in the grammar, so that generic, trait-bounded code
treats primitives and custom types uniformly:

```fig
fn sum<T: Add>(a: T, b: T) -> T { a + b }

sum(1, 2);           // needs `int` to satisfy `T: Add`
sum(p1, p2);          // needs `Point`'s own `impl Add` to satisfy it too
```

There's no fig source behind `int`'s `Add` implementation, and there never
can be: the body would have to say `fn add(self, other: int) -> int { self
+ other }`, which defines `+` in terms of itself. So unlike a custom type's
`impl`, and unlike an [ambient module](../modules/ambient-modules.md)'s
bodyless signatures, there is no declaration anywhere in fig-visible syntax
for this — no `impl` block, bodyless or otherwise, and no module path
pointing at one. The type checker simply treats "`int` and `float` satisfy
`Add`/`Sub`/`Mul`/`Div`/`Rem`/`Neg`," "`int` also satisfies the bitwise
traits," and similar facts about `bool`/`char` as built-in axioms, the same
way it looks up a real `impl` block for a user-defined type — a script
never needs to know or care which case it's hitting.

This is a different, narrower kind of "no fig source" than an ambient
module: an ambient module's implementation lives in the *host* embedding
fig and can vary by embedder; a primitive type's operator traits are
guaranteed by the fig language itself, identically everywhere fig runs,
regardless of what's hosting the script. Nothing about them belongs to, or
can be overridden by, fig-engine or any other host.

Because there's no browsable `impl` to point at, the guarantee has to be
documented directly instead. The exact trait names are still standard-
library TBD (see above), but the shapes primitives are guaranteed to
satisfy are:

| Type | Traits |
|---|---|
| `int` | arithmetic (`+ - * / %` and unary `-`), bitwise (`& \| ^ << >>` and unary `!`), equality, ordering |
| `float` | arithmetic (`+ - * / %` and unary `-`), equality, ordering |
| `bool` | logical `!`, equality |
| `char` | equality, ordering |
| `String` | `+` (concatenation), equality, ordering |

## Structs get `==` for free

This is a *different* "no `impl` needed" than primitives get, worth not
conflating with the section above: it's a real default behavior, not an
intrinsic/unoverridable one, and it applies to aggregate types you define,
not to `int`/`float`/etc.

Unlike Rust — where deriving `PartialEq` normally requires
`#[derive(PartialEq)]`, and fig has no derive macros (see
[Differences from Rust](../design/differences-from-rust.md)) — every
`struct` and `enum` in fig is structurally comparable with `==`/`!=` out of
the box, comparing field-by-field, with no `impl` needed. See
[Equality compares values, not identity](../design/values-and-mutation.md#equality-compares-values-not-identity).
Implementing the equality trait explicitly is only necessary to *customize*
equality beyond the default structural comparison.
