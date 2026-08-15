# Closures

A closure is an anonymous function that can capture variables from the
scope it's defined in, written with pipes around its parameters instead of
`fn name(...)`:

```roo
let add_one = |x| x + 1;
add_one(5); // 6
```

## Syntax

Parameter types, a return type, and a block body are all optional, in the
same "annotate what you want checked" spirit as everything else in roo —
see [Gradual Typing](../types/gradual-typing.md):

```roo
let short = |x| x + 1;
let typed = |x: int| -> int { x + 1 };
let multi_statement = |x: int| {
    let doubled = x * 2;
    doubled + 1
};
```

## Capturing the environment

A closure can refer to variables from its enclosing scope directly:

```roo
let factor = 3;
let scale = |x: int| x * factor;
scale(5); // 15
```

Captured variables follow the same rules as any other value binding or
parameter passing in roo: a captured primitive is copied into the closure,
and a captured reference type (a `struct`, array, `String`, ...) is shared
with the closure, so mutations through either the outer binding or the
closure are visible to both — see
[The Value Model](../design/values-and-mutation.md).

```roo
let counter = Counter { value: 0 };
let increment = || {
    counter.value += 1; // mutates the same Counter `counter` refers to
};
increment();
increment();
print(counter.value); // 2
```

Rust requires you to choose, via the `Fn`/`FnMut`/`FnOnce` traits and the
`move` keyword, exactly how a closure interacts with the ownership of what
it captures. roo has none of that: there is one kind of closure, and it
captures exactly the way a function parameter would receive the same
variable — see [Differences from Rust](../design/differences-from-rust.md).

## Why `fn` and closures capture differently

A named `fn` — even one nested inside a block, like
[the `helper` example](../expressions/blocks.md#nested-functions-and-types-and-hoisting)
— does **not** capture variables from its enclosing scope, only a closure
does. This isn't a leftover Rust restriction roo kept out of convention;
Rust's version of this rule is mainly about ownership bookkeeping, which
roo doesn't have. roo's reason is different, and it still applies: `fn`
items are **hoisted** (visible everywhere in their enclosing block, even
before their own declaration — see [Blocks and Scope](../expressions/blocks.md#nested-functions-and-types-and-hoisting)),
while a `let` binding only exists from its own line onward. If a hoisted
`fn` could capture a `let`, calling that `fn` from code that runs *before*
the `let` it wants to capture — which hoisting explicitly allows — would
have no well-defined value to use. There's no such problem for a closure:
a closure isn't hoisted, it's an ordinary expression evaluated in normal
execution order, so by the time a closure captures something, that
something already has a value.

So the split isn't arbitrary: **hoisted things can't capture, and things
that capture aren't hoisted.** `fn` and closures are roo's two sides of
that line, not two historical accidents that happen to look like Rust's.

## Closures as values

A closure's type is written the same way a named function's is,
`Fn(ParamTypes) -> ReturnType`, and the two are interchangeable wherever
that type is expected:

```roo
fn apply_twice(f: Fn(int) -> int, x: int) -> int {
    f(f(x))
}

apply_twice(|x| x + 1, 5); // 7
apply_twice(square, 5);     // also fine — a named `fn` has the same type
```
