# Functions

A function is declared with `fn`, a parameter list, an optional return
type, and a block body:

```fig
fn add(a: int, b: int) -> int {
    a + b
}
```

## Parameters

Each parameter is a name and an optional type annotation, separated by a
comma. Unlike a `let` binding, a parameter has no local initializer for fig
to infer a type from, so an unannotated parameter is dynamically typed
rather than a type error — see [Gradual Typing](../types/gradual-typing.md):

```fig
fn describe(name: String, value) { // `value` is dynamically typed
    print(name);
    print(value);
}
```

Writing `value: any` instead of just `value` means exactly the same thing
— see [Gradual Typing: The explicit `any` type](../types/gradual-typing.md#the-explicit-any-type)
— just spelled out.

Any pattern can appear in parameter position, not just a plain name — most
commonly to destructure a tuple or struct argument directly:

```fig
fn distance_from_origin((x, y): (float, float)) -> float {
    (x * x + y * y).sqrt()
}
```

## Return type

`-> Type` after the parameter list declares the return type, statically
checked exactly like a parameter annotation. Omitting it follows the same
[gradual typing](../types/gradual-typing.md) rule as every other
unannotated position: the return type is **dynamically typed**, not
inferred and not silently `()`. fig has no inferred return types — a
return type is either written out, or left dynamic; never guessed from the
body (see [Type Inference](../types/inference.md)).

```fig
fn log(message: String) { // return type is dynamic, not `()`
    print(message);
}
```

At runtime, `log` still produces `()` here, because its body has no
trailing expression — see
[The trailing-expression rule](../expressions/expressions-and-statements.md#the-trailing-expression-rule).
That's a fact about what the block evaluates to, completely independent of
whether the return type is statically checked. Leaving the annotation off
only means fig won't *enforce* that every call to `log` produces `()`; a
later edit that adds a trailing expression to `log`'s body wouldn't be a
type error, because there's no annotation there to violate:

```fig
fn log(message: String) { // still no annotation — still dynamic
    print(message);
    message.len() // not an error: the return type isn't statically pinned
}
```

If you want fig to actually guarantee a function returns nothing, annotate
it explicitly:

```fig
fn log(message: String) -> () {
    print(message);
}
```

Conversely, `-> any` spells out "dynamically typed" explicitly, if you'd
rather not rely on the reader knowing that's what an absent return type
already means.

## Returning a value

A function returns the value of its body block's trailing expression —
see [The trailing-expression rule](../expressions/expressions-and-statements.md#the-trailing-expression-rule)
— with no `return` needed for the common case of "the last thing this
function computes is what it returns":

```fig
fn square(n: int) -> int {
    n * n
}
```

`return expr` exits the function immediately with the given value, useful
for early returns:

```fig
fn first_positive(numbers: [int]) -> int {
    for n in numbers {
        if n > 0 {
            return n;
        }
    }
    -1
}
```

## Calling functions

Ordinary call syntax, positional arguments only — fig has no named or
default arguments, matching Rust:

```fig
add(1, 2);
```

## Functions as values

A function name refers to a callable value that can be passed around, with
type `Fn(ParamTypes) -> ReturnType`:

```fig
fn apply(f: Fn(int) -> int, x: int) -> int {
    f(x)
}

apply(square, 5); // 25
```

See [Closures](closures.md) for anonymous functions and capturing.

## Nested and recursive functions

Functions can be declared inside other functions (see
[Nested functions and types, and hoisting](../expressions/blocks.md#nested-functions-and-types-and-hoisting)) —
note that a nested `fn`, unlike a [closure](closures.md#why-fn-and-closures-capture-differently),
still can't capture its enclosing function's `let` bindings — and can call
themselves recursively:

```fig
fn factorial(n: int) -> int {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}
```

## Generic functions

A function can be parameterized over types — see
[Generics](../abstraction/generics.md):

```fig
fn largest<T: PartialOrd>(items: [T]) -> T {
    let best = items[0];
    for item in items {
        if item > best {
            best = item;
        }
    }
    best
}
```
