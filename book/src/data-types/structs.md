# Structs

A `struct` bundles named values together into a single, nominal type.
fig supports the same three struct forms Rust does.

## Named-field structs

```fig
struct Point {
    x: float,
    y: float,
}

let origin = Point { x: 0.0, y: 0.0 };
print(origin.x);
```

Field-init shorthand works when a variable's name matches the field name:

```fig
fn make_point(x: float, y: float) -> Point {
    Point { x, y } // shorthand for `Point { x: x, y: y }`
}
```

Struct update syntax builds a new value from an existing one, overriding
specific fields:

```fig
let moved = Point { x: 5.0, ..origin }; // y comes from origin
```

Fields can mix typed and untyped, exactly like function parameters — see
[Gradual Typing](../types/gradual-typing.md). `payload: any` would mean the
same thing as leaving the annotation off, just spelled out:

```fig
struct Event {
    name: String,
    payload, // dynamically typed
}
```

## Tuple structs

A struct whose fields are positional rather than named, useful for giving a
distinct type to what's conceptually a tuple:

```fig
struct Pair(int, int);

let p = Pair(3, 4);
print(p.0);
print(p.1);
```

`Pair(3, 4)` and a plain tuple `(3, 4)` are different, incompatible types
even though they hold the same shape of data — struct types in fig are
always **nominal**, never structural (see
[Gradual Typing: What gradual typing is not](../types/gradual-typing.md#what-gradual-typing-is-not)).

Each positional field has its own `pub`, the same as a named field does —
useful for a newtype-style wrapper that wants to expose some parts of
itself and keep others opaque:

```fig
struct EntityId(pub int);         // .0 is visible outside the module
struct Meters(float);              // the field is private — a pure opaque handle
```

## Unit-like structs

A struct with no fields at all, useful as a marker type:

```fig
struct Marker;

let m = Marker;
```

## Structs are reference types

Like every non-primitive type, a `struct` value follows reference
semantics: assigning or passing a struct value shares the same underlying
data rather than copying it. See
[The Value Model](../design/values-and-mutation.md) for the full rules,
including how `mut` on a binding controls whether you can mutate fields
through it.

## Methods

An `impl` block attaches functions to a struct type. A function inside an
`impl` block that takes `self` as its first parameter is a **method**,
called with `.` syntax; one that doesn't is an **associated function**,
called with `::` syntax (commonly used for constructors):

```fig
struct Point { x: float, y: float }

impl Point {
    fn new(x: float, y: float) -> Point {
        Point { x, y }
    }

    fn magnitude(self) -> float {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    fn translate(mut self, dx: float, dy: float) {
        self.x += dx;
        self.y += dy;
    }
}

let mut p = Point::new(3.0, 4.0);
print(p.magnitude()); // 5.0
p.translate(1.0, 1.0); // mutates p directly, no `&mut self` needed
```

Rust requires a method to declare whether its receiver is `self`, `&self`,
or `&mut self`, because that's where the borrow checker's rules attach.
fig only ever has `self` as the receiver's name — since a struct value is
already a reference type, `self` inside a method refers to the same value
the caller called the method on, with no separate borrowed-receiver type to
opt into. Whether the method body may write to `self`'s fields is governed
by the same `mut`-on-a-binding rule as everywhere else in fig (see
[The Value Model](../design/values-and-mutation.md#mut-still-controls-the-binding-not-the-data)):
write `mut self` to allow it, plain `self` for a read-only method. Either
way, no `&` is ever written, and calling the method looks identical from
the outside. See [Differences from Rust](../design/differences-from-rust.md).

Multiple `impl` blocks for the same struct are allowed, and traits are
implemented for a struct the same way — see [Traits](../abstraction/traits.md).

## Generic structs

A struct can be parameterized over one or more types — see
[Generics](../abstraction/generics.md):

```fig
struct Wrapper<T> {
    value: T,
}

let boxed = Wrapper { value: 5 };
```
