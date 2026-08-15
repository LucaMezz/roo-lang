# match

`match` compares a value against a series of patterns and runs the block
for the first one that matches. It is roo's (and Rust's) primary control
flow tool for working with `enum`s, and is exhaustive: every possible value
of the matched expression's type must be covered by some arm, or the
compiler rejects the `match`.

```roo
enum Direction { North, South, East, West }

fn describe(d: Direction) -> String {
    match d {
        Direction::North => "up",
        Direction::South => "down",
        Direction::East => "right",
        Direction::West => "left",
    }
}
```

## `match` as an expression

Like `if`, `match` is an expression: every arm's block is evaluated as an
expression, and every arm must produce the same type.

```roo
let size = match count {
    0 => "empty",
    1 => "one item",
    _ => "many items",
};
```

## The wildcard arm

`_` matches anything not covered by the preceding arms, and is roo's usual
way to satisfy exhaustiveness without listing every case:

```roo
match status_code {
    200 => "ok",
    404 => "not found",
    500 => "server error",
    _ => "unknown",
}
```

## Patterns

A `match` arm's pattern can be:

- **A literal**: `0`, `"hello"`, `true`
- **A binding**: a bare name, which matches anything and binds it —
  `n => print(n)`
- **The wildcard**: `_`, matches anything, binds nothing
- **A range**: `1..=5 => ...`
- **A tuple or array pattern**: `(x, y) => ...`, `[first, second] => ...`
- **A struct pattern**: `Point { x, y } => ...`, or partially,
  `Point { x, .. } => ...`
- **An enum variant pattern**: `Shape::Circle(radius) => ...`,
  `Shape::Rectangle { width, height } => ...`
- **An or-pattern**: `1 | 2 | 3 => ...`, matching any of several patterns
- **A binding with a sub-pattern**, using `@`: `n @ 1..=5 => ...`, which
  matches the range *and* binds the matched value to `n`

See [Pattern Matching](../data-types/pattern-matching.md) for the full
grammar and destructuring rules shared by `match`, `if let`, `while let`,
`let else`, and function parameters.

## Match guards

An arm can carry an additional `if` condition, checked only if the pattern
itself matches — if the guard fails, matching continues to the next arm:

```roo
match point {
    Point { x, y } if x == y => print("on the diagonal"),
    Point { x, .. } if x == 0 => print("on the y-axis"),
    _ => print("somewhere else"),
}
```

## Exhaustiveness

Because `match` must cover every case, adding a new variant to an `enum`
causes every `match` on that enum without a `_` arm to fail to compile
until the new variant is handled — a deliberate feature (shared with Rust)
that makes it hard to forget to update all the places that branch on an
enum's shape when the shape changes.
