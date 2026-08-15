# Variables

## `let` bindings

A variable is introduced with `let`, optionally with a type annotation, and
is **mutable by default** — unlike Rust, and like most scripting languages.
There is no `mut` keyword and no way to declare a binding immutable:

```fig
let x = 5;
x = 6; // ok
```

As covered in [The Value Model](../design/values-and-mutation.md), a
binding can always be reassigned, and — for reference types — mutated
through (assign to a field, index, etc.):

```fig
let point = Point { x: 0, y: 0 };
point.x = 5;                       // ok
point = Point { x: 1, y: 1 };      // ok
```

## Shadowing

A new `let` with the same name **shadows** the previous binding rather than
reassigning it — a different mechanism from plain assignment, and, as in
Rust, legal even if the new binding has a different type:

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
guaranteeing an initializer is evaluable *before the program runs at all*
— a guarantee Rust needs for things like fixed-size array lengths and
`static` initialization order, none of which fig has (see
[Differences from Rust](../design/differences-from-rust.md)). Without a
use for that guarantee, an ordinary module-level `let` covers the same
need — nothing stops it from being reassigned later, the same as any other
binding. By convention (not enforced by the compiler), a top-level binding
meant to be read as a fixed, tunable value is named in
`SCREAMING_SNAKE_CASE`, the same convention Rust uses for `const`:

```fig
let MAX_RETRIES: int = 5;
let PI: float = 3.14159265;
```

A module-level `let` is an ordinary binding like any other — it follows
the same [value model](../design/values-and-mutation.md) as a `let` inside
a function, with no special aliasing or re-evaluation rules attached to it.

### Structuring anything larger than a few lines

Because top-level code isn't inside a function, `return` and `?` don't
mean anything there — both are defined in terms of "the current function"
(see [Expressions and Statements](../expressions/expressions-and-statements.md#return-break-and-continue-are-expressions-too)
and [Error Handling](../errors/error-handling.md#the--operator)), and the
top level has none. This is a direct, unsurprising consequence of fig
having no `fn main` to begin with — not a separate restriction — but it
does mean a script that wants either has to put its logic inside a real
function and call that function explicitly:

```fig
fn run() {
    let data = load_config("settings.fig")?; // `?` needs a real function
    if data.invalid {
        return; // same for `return`
    }
    apply(data);
}

run(); // nothing invokes `run` automatically — this call is what runs it
```

For a short script, plain top-level statements are simplest and don't need
this at all. Once a script grows past that — enough to want early returns,
or fallible steps chained with `?` — wrapping its body in a function like
`run` above and calling it as the last line is the idiomatic way to get
there, not a workaround.
