# Operators

## Arithmetic

| Operator | Meaning | Operand types |
|---|---|---|
| `+` | Addition (also `String`/array concatenation) | `int`, `float`, `String`, `[T]` |
| `-` | Subtraction | `int`, `float` |
| `*` | Multiplication | `int`, `float` |
| `/` | Division | `int` (truncating), `float` |
| `%` | Remainder | `int`, `float` |
| `-` (unary) | Negation | `int`, `float` |

`int` and `float` never mix in an arithmetic expression without an explicit
[`as`](../types/casting.md) cast.

## Comparison

| Operator | Meaning |
|---|---|
| `==` | Equal (structural, see [Equality compares values, not identity](../design/values-and-mutation.md#equality-compares-values-not-identity)) |
| `!=` | Not equal |
| `<`, `<=`, `>`, `>=` | Ordering, for types that support it (`int`, `float`, `char`, `String`) |

## Logical

| Operator | Meaning |
|---|---|
| `&&` | Logical AND, short-circuiting |
| <code>&#124;&#124;</code> | Logical OR, short-circuiting |
| `!` | Logical NOT (on `bool`) |

Both operands of `&&`/`||` must be `bool` — there's no truthy/falsy
coercion of other types. Short-circuiting means the right operand isn't
evaluated if the left already determines the result:

```fig
fn expensive() -> bool { print("called"); true }

false && expensive(); // "called" is never printed
true || expensive();  // "called" is never printed
```

## Bitwise

| Operator | Meaning |
|---|---|
| `&` | Bitwise AND |
| <code>&#124;</code> | Bitwise OR |
| `^` | Bitwise XOR |
| `!` | Bitwise NOT (on `int`) |
| `<<` | Left shift |
| `>>` | Right shift |

`!` is overloaded the same way it is in Rust: logical NOT on `bool`,
bitwise NOT on `int`. There is no separate `~` operator.

## Assignment

| Operator | Meaning |
|---|---|
| `=` | Assignment |
| `+=` `-=` `*=` `/=` `%=` | Compound arithmetic assignment |
| `&=` <code>&#124;=</code> `^=` `<<=` `>>=` | Compound bitwise assignment |

Assignment is a statement-position construct, not an expression that
produces a chainable value — `a = b = c` is not valid fig, matching Rust.

## Range

| Operator | Meaning |
|---|---|
| `..` | Exclusive range (`0..5` is `0, 1, 2, 3, 4`) |
| `..=` | Inclusive range (`0..=5` is `0, 1, 2, 3, 4, 5`) |

Ranges are mainly used in `for` loops and `match` patterns — see
[Loops](../control-flow/loops.md) and
[Pattern Matching](../data-types/pattern-matching.md).

## Member access and paths

| Operator | Meaning |
|---|---|
| `.` | Field/method access (`point.x`, `point.magnitude()`) |
| `::` | Path separator, for modules and associated items (`Shape::Circle`, `std::io`) |

## Casting

| Operator | Meaning |
|---|---|
| `as` | Explicit type conversion — see [Casting and Conversion](../types/casting.md) |

## Error propagation

| Operator | Meaning |
|---|---|
| `?` | Propagate an error/`None` out of the current function — see [Error Handling](../errors/error-handling.md) |

## What's *not* here

`&` and `&mut` as **unary, prefix** operators (address-of/borrow) don't
exist in fig — `&` only ever appears as the binary bitwise-AND operator.
`*` as a **unary, prefix** operator (dereference) doesn't exist either —
`*` only ever appears as binary multiplication. Both are consequences of
fig having no references; see
[Differences from Rust](../design/differences-from-rust.md).

See [Operator Precedence](../appendix/operator-precedence.md) for the full
precedence table.
