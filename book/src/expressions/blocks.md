# Blocks and Scope

A block is a sequence of statements, optionally ending in a trailing
expression, surrounded by `{ }`. As covered in
[Expressions and Statements](expressions-and-statements.md), a block is
itself an expression: it evaluates to its trailing expression's value, or
`()` if there isn't one.

```fig
let result = {
    let a = 1;
    let b = 2;
    a + b // trailing expression, no `;` — this is the block's value
};
print(result); // 3
```

## Scope

Every block introduces a new scope. A `let` inside a block is only visible
from that point until the end of the block:

```fig
let x = 1;
{
    let y = 2;
    print(x + y); // 3 — x is visible from the outer scope
}
print(y); // error: `y` is not defined here — it went out of scope
```

Function bodies, `if`/`else` branches, `match` arms, and loop bodies are all
blocks, and each introduces its own scope the same way.

## Nested functions and types

A block can contain `fn`, `struct`, `enum`, `trait`, `impl`, and `mod`
declarations, not just `let`s — these are visible throughout the enclosing
block (including before the point they're declared textually), but not
outside it:

```fig
fn outer() -> int {
    fn helper(n: int) -> int {
        n * 2
    }
    helper(21)
}
```

## Blocks as arguments to control flow

Because `if`, `match`, `loop`, `while`, and `for` all take a block as their
body, and blocks are expressions, control-flow constructs compose the way
they do in Rust — see [if and if let](../control-flow/if.md),
[match](../control-flow/match.md), and [Loops](../control-flow/loops.md).
