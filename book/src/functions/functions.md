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
comma. Unlike a Rust parameter, an unannotated one isn't an error, and
unlike a dynamically-typed one, it isn't unchecked either — fig infers it
from how the parameter is actually used in the function body, generalizing
it into a genuine generic type parameter if nothing pins it down to
anything more specific. See [Type Inference](../types/inference.md) for
the full mechanics; the short version:

```fig
fn describe(name: String, value) {
    print(name);
    print(value);
}
```

infers to `fn describe<T>(name: String, value: T)` — `value` becomes a
real, checked generic parameter, not a dynamically-typed one. Writing
`value: any` instead of just `value` opts out of that inference and asks
for genuinely dynamic behavior — see
[Gradual Typing: The explicit `any` type](../types/gradual-typing.md#the-explicit-any-type).

Any pattern can appear in parameter position, not just a plain name — most
commonly to destructure a tuple or struct argument directly:

```fig
fn distance_from_origin((x, y): (float, float)) -> float {
    (x * x + y * y).sqrt()
}
```

## Return type

`-> Type` after the parameter list declares the return type, statically
checked exactly like a parameter annotation. Omitting it doesn't mean
dynamically typed and doesn't mean silently `()` either — fig **infers**
the return type from the body, the same way it infers an untyped
parameter (see [Type Inference](../types/inference.md)):

```fig
fn log(message: String) { // inferred: -> ()
    print(message);
}
```

`log`'s body has no trailing expression, so the block it evaluates to is
`()` — see
[The trailing-expression rule](../expressions/expressions-and-statements.md#the-trailing-expression-rule)
— and fig infers the return type to be exactly that, `()`, checked from
here on exactly as if you'd written `-> ()` yourself. That means a later
edit that adds a trailing expression to the body is no longer a
consequence-free change: it can turn the inferred return type into
something else, which is a real type error anywhere the *old* inferred
type had already been relied on:

```fig
fn log(message: String) { // now inferred: -> String
    print(message);
    message // trailing expression added — the inferred return type changes
}
```

If you want fig to guarantee a function's return type regardless of how
its body is later edited, annotate it explicitly — this is the same
tradeoff any inferred type has, not something specific to return types:

```fig
fn log(message: String) -> () {
    print(message);
}
```

Writing `-> any` opts out of return-type inference entirely and asks for
a genuinely dynamic return value instead, checked at each call site
rather than up front — see
[Gradual Typing: The same boundary applies to a function's return type](../types/gradual-typing.md#the-same-boundary-applies-to-a-functions-return-type).

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

`largest` needs `<T: PartialOrd>` written out, since a trait bound is a
promise the checker has to be told about before it can check the body
against it. A simpler function that doesn't need a bound, like `identity`
from [Type Inference](../types/inference.md#untyped-functions-are-inferred-and-generalized-not-dynamically-typed),
doesn't need `<T>` written at all — fig infers it, arriving at the exact
same generic type either way.
