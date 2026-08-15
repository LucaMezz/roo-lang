# Structs

A `struct` bundles named values together into a single, nominal type.
roo supports the same three struct forms Rust does.

## Named-field structs

```roo
struct Point {
    x: float,
    y: float,
}

let origin = Point { x: 0.0, y: 0.0 };
print(origin.x);
```

Field-init shorthand works when a variable's name matches the field name:

```roo
fn make_point(x: float, y: float) -> Point {
    Point { x, y } // shorthand for `Point { x: x, y: y }`
}
```

Struct update syntax builds a new value from an existing one, overriding
specific fields:

```roo
let moved = Point { x: 5.0, ..origin }; // y comes from origin
```

Fields can mix typed and untyped, the same way a function's parameters
can — but unlike an untyped parameter, an untyped field isn't inferred
from usage; there's no single "the field is constructed here" site the
way there's a single function body to check. It's genuinely dynamically
typed instead — see [Gradual Typing](../types/gradual-typing.md) and
[Type Inference: Where inference does not apply yet](../types/inference.md#where-inference-does-not-apply-yet).
`payload: any` would mean the same thing as leaving the annotation off,
just spelled out:

```roo
struct Event {
    name: String,
    payload, // dynamically typed
}
```

## Tuple structs

A struct whose fields are positional rather than named, useful for giving a
distinct type to what's conceptually a tuple:

```roo
struct Pair(int, int);

let p = Pair(3, 4);
print(p.0);
print(p.1);
```

`Pair(3, 4)` and a plain tuple `(3, 4)` are different, incompatible types
even though they hold the same shape of data — struct types in roo are
always **nominal**, never structural (see
[Gradual Typing: What gradual typing is not](../types/gradual-typing.md#what-gradual-typing-is-not)).

Each positional field has its own `pub`, the same as a named field does —
useful for a newtype-style wrapper that wants to expose some parts of
itself and keep others opaque:

```roo
struct EntityId(pub int);         // .0 is visible outside the module
struct Meters(float);              // the field is private — a pure opaque handle
```

## Unit-like structs

A struct with no fields at all, useful as a marker type:

```roo
struct Marker;

let m = Marker;
```

## Structs are reference types

Like every non-primitive type, a `struct` value follows reference
semantics: assigning or passing a struct value shares the same underlying
data rather than copying it. See
[The Value Model](../design/values-and-mutation.md) for the full rules,
including how every binding can mutate fields through it.

## Methods

An `impl` block attaches functions to a struct type. A function inside an
`impl` block that takes `self` as its first parameter is a **method**,
called with `.` syntax; one that doesn't is an **associated function**,
called with `::` syntax (commonly used for constructors):

```roo
struct Point { x: float, y: float }

impl Point {
    fn new(x: float, y: float) -> Point {
        Point { x, y }
    }

    fn magnitude(self) -> float {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    fn translate(self, dx: float, dy: float) {
        self.x += dx;
        self.y += dy;
    }
}

let p = Point::new(3.0, 4.0);
print(p.magnitude()); // 5.0
p.translate(1.0, 1.0); // mutates p directly, no `&mut self` needed
```

Rust requires a method to declare whether its receiver is `self`, `&self`,
or `&mut self`, because that's where the borrow checker's rules attach.
roo only ever has `self` as the receiver's name — since a struct value is
already a reference type, `self` inside a method refers to the same value
the caller called the method on, with no separate borrowed-receiver type to
opt into, and no `mut` to write either: every method body may write to
`self`'s fields, unconditionally. No `&` is ever written, and calling the
method looks identical from the outside. See
[Differences from Rust](../design/differences-from-rust.md).

Multiple `impl` blocks for the same struct are allowed, and traits are
implemented for a struct the same way — see [Traits](../abstraction/traits.md).

## Generic structs

A struct can be parameterized over one or more types — see
[Generics](../abstraction/generics.md):

```roo
struct Wrapper<T> {
    value: T,
}

let boxed = Wrapper { value: 5 };
```
