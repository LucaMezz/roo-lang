# if and if let

## `if`/`else`

`if` takes a `bool` condition — no parentheses required around it, and no
implicit truthiness for other types — followed by a block:

```roo
if temperature > 30 {
    print("hot");
} else if temperature > 15 {
    print("mild");
} else {
    print("cold");
}
```

## `if` as an expression

Every branch of an `if` can be used as an expression, provided every branch
is present (an `if` with no `else` always has type `()`, since the "missing"
branch implicitly evaluates to `()`) and every branch's trailing expression
has the same type:

```roo
let description = if temperature > 30 {
    "hot"
} else if temperature > 15 {
    "mild"
} else {
    "cold"
}; // note the semicolon — this whole if/else is one expression
```

This is roo's (and Rust's) replacement for a ternary operator — there is no
separate `cond ? a : b` syntax.

## `if let`

`if let` tests whether a value matches a single pattern, binding any
variables the pattern introduces for use in the following block — the
concise form of a `match` with one meaningful arm and a wildcard fallback.
See [Pattern Matching](../data-types/pattern-matching.md) for the full
pattern grammar.

```roo
if let Some(value) = maybe_value {
    print(value);
} else {
    print("nothing there");
}
```

`if let` can be chained with `else if let`, and mixed with a plain `else`:

```roo
if let Circle(radius) = shape {
    print(radius);
} else if let Rectangle { width, height } = shape {
    print(width * height);
} else {
    print("unknown shape");
}
```

## `let else`

`let else` is the inverse shape: it binds a pattern's variables into the
**surrounding** scope on a match, and requires the `else` block to
diverge (`return`, `break`, `continue`, or `panic`) on a non-match, so
the bound variables are guaranteed to exist afterward:

```roo
fn first_word(text: String) -> String {
    let Some(word) = text.split_first() else {
        return "";
    };
    word // `word` is usable here, unlike with `if let`
}
```

This avoids the extra nesting an equivalent `if let ... else { return ... }`
followed by using the binding outside the block would otherwise require.
