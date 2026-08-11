# Enums

An `enum` defines a type by enumerating its possible variants. Unlike
enums in most C-family languages, and exactly like Rust, a fig `enum`
variant can carry data, and different variants of the same enum can carry
different, independently-shaped data.

## Fieldless variants

```fig
enum Direction {
    North,
    South,
    East,
    West,
}

let heading = Direction::North;
```

## Variants with data

A variant can carry positional data (like a tuple struct) or named fields
(like a struct), and different variants can mix both styles:

```fig
enum Shape {
    Circle(float),                       // tuple-style: one field
    Rectangle { width: float, height: float }, // struct-style: named fields
    Point,                                // no data at all
}

let a = Shape::Circle(2.0);
let b = Shape::Rectangle { width: 3.0, height: 4.0 };
let c = Shape::Point;
```

## Working with enum values

`match` is the primary tool for inspecting an enum value and extracting its
data — see [match](../control-flow/match.md) and
[Pattern Matching](pattern-matching.md):

```fig
fn area(shape: Shape) -> float {
    match shape {
        Shape::Circle(radius) => 3.14159 * radius * radius,
        Shape::Rectangle { width, height } => width * height,
        Shape::Point => 0.0,
    }
}
```

## Enums are reference types

Like structs, an enum value follows reference semantics — see
[The Value Model](../design/values-and-mutation.md).

## Methods

Enums can have `impl` blocks exactly like structs, including methods and
associated functions:

```fig
impl Shape {
    fn area(self) -> float {
        match self {
            Shape::Circle(radius) => 3.14159 * radius * radius,
            Shape::Rectangle { width, height } => width * height,
            Shape::Point => 0.0,
        }
    }
}

let a = Shape::Circle(2.0);
print(a.area());
```

## Generic enums

Like structs and functions, enums can be parameterized over types — see
[Generics](../abstraction/generics.md). This is exactly how an `Option`-
or `Result`-shaped standard-library type would be defined, once the
standard library exists (see [Error Handling](../errors/error-handling.md)):

```fig
enum Option<T> {
    Some(T),
    None,
}
```

## What's *not* here

Rust allows fieldless enums to specify an explicit numeric discriminant
(`enum Status { Ok = 200, NotFound = 404 }`), primarily so the value can be
cast to an integer matching some external (often C/FFI) representation. fig
omits this — see
[Differences from Rust](../design/differences-from-rust.md) — since it has
no FFI story for that representation to matter to.
