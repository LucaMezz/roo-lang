# Variables

## `let` bindings

A variable is introduced with `let`, optionally with a type annotation, and
is **mutable by default** — unlike Rust, and like most scripting languages.
There is no `mut` keyword and no way to declare a binding immutable:

```roo
let x = 5;
x = 6; // ok
```

As covered in [The Value Model](../design/values-and-mutation.md), a
binding can always be reassigned, and — for reference types — mutated
through (assign to a field, index, etc.):

```roo
let point = Point { x: 0, y: 0 };
point.x = 5;                       // ok
point = Point { x: 1, y: 1 };      // ok
```

## Shadowing

A new `let` with the same name **shadows** the previous binding rather than
reassigning it — a different mechanism from plain assignment, and, as in
Rust, legal even if the new binding has a different type:

```roo
let spaces = "   ";
let spaces = spaces.len(); // shadows the String with an int
```

Shadowing inside a nested block only lasts for that block:

```roo
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

```roo
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

## No module-level bindings

`let` only works inside a function or closure body — the same restriction
Rust has, not a looser one. Each roo file is itself a module (see
[Modules and Visibility](../modules/modules.md)), and a module's top level
— along with the body of an inline `mod { ... }` block — only accepts
**items**: `fn`, `struct`, `enum`, `trait`, `impl`, `mod`, and `use`. `let`
is not an item, so it can't be written directly inside one:

```roo
mod shapes {
    let PI: float = 3.14159265; // error: `let` isn't allowed here —
                                  // only items are
}
```

A direct consequence: a roo file has no implicit top-to-bottom execution
either. A module made of nothing but items doesn't *do* anything by
itself — there's no script-style "run every statement in order" the way a
Lua/Luau chunk works. roo has no `fn main` and no other reserved
entry-point name. Running a program means something *outside* the module
calls into one of its functions: the host embedding roo (see
[Ambient Modules](../modules/ambient-modules.md)), or another roo module
that `use`s it and calls a function explicitly. Which function that is,
and when it's called, is entirely up to whatever's driving the program —
the language doesn't pick one for you.

### No module-level constants, for now

The sharpest edge of this: without a module-level `let`, there's currently
no way to write a single named, fixed value at module scope and read it
back as `shapes::PI`, the way Rust's `pub const PI: f64 = 3.14159265;`
works. A zero-argument function is the closest thing roo has today:

```roo
mod shapes {
    pub fn pi() -> float {
        3.14159265
    }
}

let area = shapes::pi() * radius * radius;
```

This was a deliberate simplification, weighed and accepted, not an
oversight. roo's `const` keyword was already unnecessary as *Rust's*
`const` — the guarantee it protects, an initializer evaluable *before the
program runs at all* (backing fixed-size array lengths, `static`
initialization order, and the like), has no use in roo, since roo has no
compile-time evaluation at all (see
[Differences from Rust](../design/differences-from-rust.md)). Once a
module only holds items — a simpler, more uniform rule than "items, plus
`let`, but only at this one scope" — reintroducing a separate binding form
just to name a fixed module-scoped value didn't earn its keep either: the
scenarios that actually need a bare constant over a one-line accessor
function are rare enough in a scripting language that the simplicity won.
`const` stays reserved (see [Keywords](../appendix/keywords.md)) in case
that judgment changes later.

## Structuring anything larger than a few lines

Because every function is a real function — never top-level script code —
`return` and `?` always mean something, and always refer to the function
they're written in (see
[Expressions and Statements](../expressions/expressions-and-statements.md#return-break-and-continue-are-expressions-too)
and [Error Handling](../errors/error-handling.md#the--operator)). There's
no separate "top-level code" case to worry about falling outside that rule,
the way there would be in a language that lets you write bare statements
at module scope:

```roo
pub fn run() {
    let data = load_config("settings.roo")?; // `?` needs a real function
    if data.invalid {
        return; // same for `return`
    }
    apply(data);
}
```

Something else — the host embedding roo, or another module that `use`s
this one — calls `run()`; nothing in this file does.
