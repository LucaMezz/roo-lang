# Closures

A closure is an anonymous function that can capture variables from the
scope it's defined in, written with pipes around its parameters instead of
`fn name(...)`:

```fig
let add_one = |x| x + 1;
add_one(5); // 6
```

## Syntax

Parameter types, a return type, and a block body are all optional, in the
same "annotate what you want checked" spirit as everything else in fig —
see [Gradual Typing](../types/gradual-typing.md):

```fig
let short = |x| x + 1;
let typed = |x: int| -> int { x + 1 };
let multi_statement = |x: int| {
    let doubled = x * 2;
    doubled + 1
};
```

## Capturing the environment

A closure can refer to variables from its enclosing scope directly:

```fig
let factor = 3;
let scale = |x: int| x * factor;
scale(5); // 15
```

Captured variables follow the same rules as any other value binding or
parameter passing in fig: a captured primitive is copied into the closure,
and a captured reference type (a `struct`, array, `String`, ...) is shared
with the closure, so mutations through either the outer binding or the
closure are visible to both, provided the relevant binding is `mut` — see
[The Value Model](../design/values-and-mutation.md).

```fig
let mut counter = Counter { value: 0 };
let increment = || {
    counter.value += 1; // mutates the same Counter `counter` refers to
};
increment();
increment();
print(counter.value); // 2
```

Rust requires you to choose, via the `Fn`/`FnMut`/`FnOnce` traits and the
`move` keyword, exactly how a closure interacts with the ownership of what
it captures. fig has none of that: there is one kind of closure, and it
captures exactly the way a function parameter would receive the same
variable — see [Differences from Rust](../design/differences-from-rust.md).

## Closures as values

A closure's type is written the same way a named function's is,
`Fn(ParamTypes) -> ReturnType`, and the two are interchangeable wherever
that type is expected:

```fig
fn apply_twice(f: Fn(int) -> int, x: int) -> int {
    f(f(x))
}

apply_twice(|x| x + 1, 5); // 7
apply_twice(square, 5);     // also fine — a named `fn` has the same type
```
