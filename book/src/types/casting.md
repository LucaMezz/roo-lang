# Casting and Conversion

fig performs no implicit type conversions. Every conversion between
distinct types is written explicitly with the `as` operator, exactly as in
Rust.

```fig
let count: int = 5;
let ratio: float = count as float; // 5.0

let precise: float = 3.9;
let truncated: int = precise as int; // 3 — truncates toward zero, not rounds
```

## Supported casts

| From | To | Behavior |
|---|---|---|
| `int` | `float` | Exact (within `float`'s precision). |
| `float` | `int` | Truncates the fractional part toward zero. |
| `char` | `int` | Yields the Unicode scalar value's code point. |
| `int` | `char` | Valid only for values that are valid Unicode scalar values; invalid values are a runtime error. |
| `bool` | `int` | `false as int` is `0`, `true as int` is `1`. |

There is no direct `int as bool` or numeric-to-`String` cast via `as`.

## Converting to `String`

Turning a value into text isn't a cast — it goes through a trait instead,
conceptually shaped like:

```fig
trait ToString {
    fn to_string(self) -> String;
}
```

```fig
let level: int = 5;
let message = "Level " + level.to_string();
```

Exactly like the arithmetic and comparison traits (see
[Primitive types implement these traits intrinsically](../abstraction/operator-overloading.md#primitive-types-implement-these-traits-intrinsically)),
`int`, `float`, `bool`, and `char` implement `ToString` intrinsically —
guaranteed by the fig language itself, with no fig source behind it, since
it's provided by whatever's running the script rather than written out
as a real `impl` block anywhere. Unlike the arithmetic traits, this one
isn't a hard bootstrapping problem the way `int`'s own `Add` impl is (you
could, in principle, write an int-to-string conversion in terms of `%`,
`/`, and `char`s) — it's treated the same way mainly for consistency, not
because fig source is fundamentally incapable of it.

The exact trait name and method are standard-library TBD, same as the
operator traits — this section describes the guarantee (every primitive
can produce a `String`), not the final name.

## Why `as`, not implicit coercion

Implicit numeric coercion (silently treating an `int` as a `float` in
arithmetic, or a non-`bool` as truthy in a condition) is a common source of
subtle bugs in dynamically-typed scripting languages. fig follows Rust here
even though it's a scripting language: mixing `int` and `float` in an
expression without an explicit `as` is a type error, not a silent widening.

```fig
let n: int = 5;
let f: float = 1.5;
let total = n + f; // type error: `int` + `float`
let total = n as float + f; // ok
```

This rule only applies to **statically-typed** values. A dynamically-typed
(unannotated) value is checked for a valid operation when the operator
actually runs, per the gradual-typing boundary rules in
[Gradual Typing](gradual-typing.md) — but that's a *dynamic* check
happening at the boundary of the type system, not an implicit conversion
within it.
