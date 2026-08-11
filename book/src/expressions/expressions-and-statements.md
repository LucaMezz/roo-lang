# Expressions and Statements

Like Rust, fig is **expression-oriented**: most of the language produces a
value, including constructs that look like "statements" in C-family
languages — `if`, `match`, and blocks are all expressions.

## Statements

fig has two kinds of statements:

- **Declaration statements** — `let` and item declarations (`fn`, `struct`,
  `enum`, `trait`, `impl`, `mod`).
- **Expression statements** — any expression followed by a `;`, evaluated
  for its side effects, with its value discarded.

```fig
let x = 5;      // declaration statement
x + 1;           // expression statement — computes 3, discards it
```

## Expressions

Everything else is an expression, including ones that read like control
flow:

```fig
let status = if ready { "go" } else { "wait" }; // if is an expression

let described = match code {
    0 => "ok",
    _ => "error",
}; // match is an expression

let sum = {
    let a = 1;
    let b = 2;
    a + b
}; // a block is an expression, see Blocks and Scope
```

See [if and if let](../control-flow/if.md), [match](../control-flow/match.md),
and [Blocks and Scope](blocks.md) for the full rules governing each of
these as expressions.

## The trailing-expression rule

Inside a block `{ ... }`, if the final line has **no** trailing `;`, it is
the block's value; if it does (or the block is empty), the block's value is
`()`:

```fig
fn square(n: int) -> int {
    n * n       // no semicolon — this is the returned value
}

fn log_and_discard(n: int) {
    n * n;      // semicolon — value discarded, function returns ()
}
```

This rule describes what a block evaluates to when execution reaches its
closing `}` *normally*. It doesn't apply to a statement that unconditionally
diverges — most commonly `return expr;` — even though that statement also
ends in a `;`:

```fig
fn foo() -> int {
    return 5; // ends in `;`, but that's irrelevant here
}
```

`foo` returns `5`, not `()`. `return` exits the function immediately, so
the block's own value is never computed on that path at all — the `;`
after `return 5` is just ordinary statement syntax, not a signal that gets
applied to the `5`. This isn't a special case bolted onto the rule; it
falls out of `return` having its own type, `!`, covered next.

This single rule is also how a function returns a value without a `return`
keyword: a function's body is a block, and the block's trailing expression
becomes the return value. See [Functions](../functions/functions.md).

## `return`, `break`, and `continue` are expressions too

Like Rust, `return expr`, `break expr`, and `continue` are themselves
expressions — with type `!` ("never"), meaning they never actually produce
a value because control flow doesn't continue past them. This lets them
appear in any expression position, most commonly as one arm of an `if` or
`match` whose other arms produce a real value:

```fig
let value = if let Some(v) = maybe_value {
    v
} else {
    return; // valid here: `return` unifies with any type, since it never
             // actually evaluates to one
};
```

This is also the precise reason the trailing-expression rule doesn't apply
to a block ending in `return expr;` (see
[above](#the-trailing-expression-rule)): a `!`-typed expression never
finishes evaluating, so control never reaches the point where "what is
this block's value" would even be asked. The block doesn't produce `()`
on that path — it doesn't produce anything, because it never gets that
far. What the *function* returns comes from `return`'s own operand
instead, entirely bypassing the block's trailing-expression mechanism.
