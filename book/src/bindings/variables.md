# Variables

## `let` bindings

A variable is introduced with `let`, optionally with a type annotation, and
is **immutable by default** — exactly as in Rust, and unlike most scripting
languages:

```fig
let x = 5;
x = 6; // error: cannot assign twice to immutable variable `x`
```

## `mut`

Add `mut` to allow reassignment:

```fig
let mut x = 5;
x = 6; // ok
```

As covered in [The Value Model](../design/values-and-mutation.md), `mut`
controls two things for a given binding: whether the binding itself can be
reassigned, and — for reference types — whether you can mutate through it
(assign to a field, index, etc.):

```fig
let mut point = Point { x: 0, y: 0 };
point.x = 5;                       // ok: point is mut
point = Point { x: 1, y: 1 };      // ok: point is mut
```

## Shadowing

A new `let` with the same name **shadows** the previous binding rather than
mutating it — a different mechanism from `mut`, and, as in Rust, legal even
if the new binding has a different type:

```fig
let spaces = "   ";
let spaces = spaces.len(); // shadows the String with an int
```

Shadowing inside a nested block only lasts for that block:

```fig
let x = 5;
{
    let x = x * 2;
    print(x); // 10
}
print(x); // 5 — the inner shadow ended with the block
```

## Declaration without initialization

A `let` can omit the initializer, as long as the compiler can prove the
variable is assigned before it's read. This requires an explicit type
annotation if you want the binding statically typed (see
[Type Inference](../types/inference.md), since there's no initializer to
infer from):

```fig
let x: int;
if condition {
    x = 1;
} else {
    x = 2;
}
print(x); // ok — assigned on every path before use
```

## Scope

A variable is in scope from its `let` statement to the end of the
innermost enclosing block (see [Blocks and Scope](../expressions/blocks.md)),
exactly as in Rust.

## Module-level bindings

Unlike Rust, `let` isn't restricted to function bodies — a fig file runs
as a script, executing its statements top to bottom the way a Lua/Luau
chunk does, rather than requiring an explicit entry point function. `let`
works identically at the top level of a file as it does anywhere else,
including plain, unannotated, and shadowed bindings:

```fig
let max_retries: int = 5;
let greeting = "hello";
```

fig has no separate `const` keyword. Rust's `const` earns its keep by
guaranteeing an initializer is evaluable *before the program runs at all* —
a guarantee Rust needs for things like fixed-size array lengths and
`static` initialization order, none of which fig has (see
[Differences from Rust](../design/differences-from-rust.md)). Without a
use for that guarantee, an immutable, module-level `let` covers the same
need. By convention (not enforced by the compiler), a top-level binding
meant to be read as a fixed, tunable value is named in
`SCREAMING_SNAKE_CASE`, the same convention Rust uses for `const`:

```fig
let MAX_RETRIES: int = 5;
let PI: float = 3.14159265;
```

A module-level `let` is an ordinary binding like any other — it follows
the same [value model](../design/values-and-mutation.md) as a `let` inside
a function, with no special aliasing or re-evaluation rules attached to it.
