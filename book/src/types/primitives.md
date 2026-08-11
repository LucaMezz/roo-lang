# Primitive Types

fig has four primitive types. All four are **value types** — see
[The Value Model](../design/values-and-mutation.md) — copied on assignment
and when passed to functions, with no sharing between copies.

## `bool`

Either `true` or `false`. Produced by comparison and logical operators, and
the only type allowed as an `if`/`while` condition — there is no
implicit "truthiness" conversion from other types.

```fig
let ready: bool = true;
let done = 3 > 2; // bool, inferred
```

## `int`

A signed integer.

```fig
let count: int = 42;
let negative = -17;
```

Rust exposes eleven integer types (`i8` through `i128`, `isize`, and their
`u`-prefixed unsigned counterparts) so that a systems programmer can pick an
exact bit width and signedness for memory-layout or performance reasons.
fig collapses all of that to a single `int` type, because choosing an exact
width is precisely the kind of low-level, memory-layout concern fig omits —
see [Differences from Rust](../design/differences-from-rust.md). Integer
overflow behavior (wrapping, saturating, or a runtime error) is left to the
implementation to finalize; script authors should not rely on any particular
overflow behavior.

There is no separate unsigned integer type. Values that are conceptually
"always non-negative" (array lengths, indices) are just `int`s that happen
to never go negative, checked at the point of use (e.g., indexing with a
negative `int` is a runtime error) rather than by a distinct type.

## `float`

A 64-bit IEEE‑754 floating-point number, equivalent to Rust's `f64`.

```fig
let pi: float = 3.14159;
let ratio = 1.0 / 3.0;
```

As in Rust, `int` and `float` are distinct types with no implicit
conversion between them — `1 + 1.0` is a type error. Convert explicitly with
[`as`](casting.md).

fig does not have a separate `f32` — see
[Differences from Rust](../design/differences-from-rust.md) for the same
reasoning as `int`.

## `char`

A single Unicode scalar value (not a byte), exactly like Rust's `char` —
four bytes, always a valid Unicode code point.

```fig
let initial: char = 'A';
let emoji = '🦀';
```

## Default values

fig has no implicit zero-initialization — every `let` binding requires
either an explicit initializer or a later assignment before it's used,
exactly as in Rust. There is no primitive-type default akin to `0`/`false`
being assumed for an unwritten variable.
