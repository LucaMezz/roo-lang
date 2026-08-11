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

## Nested functions and types, and hoisting

A block can contain `fn`, `struct`, `enum`, `trait`, `impl`, and `mod`
declarations, not just `let`s — collectively, **items**. Items are
**hoisted**: every item in a block is visible (nameable, callable)
anywhere in that block, including *before* the line it's declared on, not
just after it. This is different from a `let`, which is only in scope from
its own line onward (see [Scope](#scope) above) — a block's items are all
resolved up front, as if the whole block "knew about" every item in it
before running any of the block's statements, regardless of the order they
appear in:

```fig
fn outer() -> int {
    let result = helper(21); // calling `helper` before its own
                               // declaration below — fine, it's hoisted

    fn helper(n: int) -> int {
        n * 2
    }

    result
}
```

Items are visible throughout the block they're declared in, but not
outside it — hoisting only reaches to the edges of the enclosing block,
never further out.

Hoisting is why a named `fn` can't capture a `let` from its enclosing
scope the way a [closure](../functions/closures.md#why-fn-and-closures-capture-differently)
can: a `let` only has a value from its own line onward, but a hoisted `fn`
can be reached from code that runs earlier than that — there's no single,
well-defined moment at which such a capture could happen.

## Blocks as arguments to control flow

Because `if`, `match`, `loop`, `while`, and `for` all take a block as their
body, and blocks are expressions, control-flow constructs compose the way
they do in Rust — see [if and if let](../control-flow/if.md),
[match](../control-flow/match.md), and [Loops](../control-flow/loops.md).
