# Pattern Matching

A **pattern** describes a shape a value might have, optionally binding parts
of it to new names. Patterns appear in more places than just `match`:

- `let` and `let else` bindings
- `if let` / `while let` conditions
- function parameters
- `for` loop variables
- `match` arms

This chapter is the shared reference for pattern syntax; see
[match](../control-flow/match.md), [if and if let](../control-flow/if.md),
and [Loops](../control-flow/loops.md) for how each construct uses patterns.

## Literal patterns

Match an exact value:

```fig
match n {
    0 => "zero",
    1 => "one",
    _ => "many",
}
```

## Binding patterns

A bare identifier matches anything and binds the matched value to that
name. Every binding produced by a pattern is reassignable, and (for a
reference type) writable-through — the same
[mutability every other binding has](../design/values-and-mutation.md#every-binding-is-mutable),
with no `mut` keyword needed to opt in:

```fig
match shape {
    Shape::Circle(radius) => print(radius), // `radius` bound here
    _ => {}
}

if let Option::Some(p) = position {
    p.x += 1.0; // ok
}

let (count, total) = (0, 100); // both reassignable
```

## The wildcard pattern

`_` matches anything and binds nothing — used to ignore a value entirely:

```fig
let (_, y) = point; // ignore the first element, bind the second
```

## Range patterns

```fig
match grade {
    90..=100 => "A",
    80..=89 => "B",
    _ => "C or below",
}
```

## Tuple and array patterns

```fig
let (x, y) = (1, 2);

match pair {
    (0, 0) => "origin",
    (x, 0) => print(x), // matches any (x, 0), binds x
    (_, _) => "elsewhere",
}

match numbers {
    [] => "empty",
    [only] => "one element",
    [first, second] => "two elements",
    _ => "more than two",
}
```

## Struct patterns

Destructure by field name; `..` ignores any remaining fields:

```fig
struct Point { x: int, y: int }

let Point { x, y } = point;

match point {
    Point { x: 0, y: 0 } => "origin",
    Point { x, .. } if x == 0 => "on the y-axis",
    Point { x, y } => print(x + y),
}
```

## Enum variant patterns

```fig
match shape {
    Shape::Circle(radius) => ...,
    Shape::Rectangle { width, height } => ...,
    Shape::Point => ...,
}
```

## Or-patterns

`|` matches if any of several patterns match:

```fig
match c {
    'a' | 'e' | 'i' | 'o' | 'u' => "vowel",
    _ => "consonant",
}
```

## `@` bindings

Binds the matched value to a name *while also* checking it against a
sub-pattern:

```fig
match age {
    n @ 0..=12 => print(n), // binds n, and requires 0..=12
    n @ 13..=19 => "teenager",
    _ => "adult",
}
```

## Refutable vs. irrefutable patterns

A pattern that can always match, given a value of the right type — a bare
binding, `_`, or a tuple/struct pattern of bindings — is **irrefutable**,
and is the only kind allowed in a `let`, function parameter, or `for` loop
variable, all of which require the match to always succeed:

```fig
let (x, y) = point; // ok — always matches a (T, T) tuple
```

A pattern that might not match — a literal, a specific enum variant, a
range — is **refutable**, and is only allowed where "doesn't match" has
somewhere to go: `match` (another arm), `if let`/`while let` (the `else`
branch or loop exit), or `let else` (the required `else` block):

```fig
let Shape::Circle(radius) = shape; // error: refutable pattern in `let` —
                                     // `shape` might not be a Circle

if let Shape::Circle(radius) = shape { // ok — non-match falls through
    print(radius);
}
```
