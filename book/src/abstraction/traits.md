# Traits

A `trait` defines a set of methods a type can implement, giving fig a way
to write code against "any type that can do X" rather than one concrete
type. Traits are how fig expresses interfaces/protocols, exactly as in
Rust.

## Defining a trait

```fig
trait Shape {
    fn area(self) -> float;
    fn perimeter(self) -> float;
}
```

## Implementing a trait

```fig
struct Circle { radius: float }

impl Shape for Circle {
    fn area(self) -> float {
        3.14159 * self.radius * self.radius
    }

    fn perimeter(self) -> float {
        2.0 * 3.14159 * self.radius
    }
}
```

A type can implement any number of traits, in any number of separate `impl`
blocks.

## Default methods

A trait method can provide a default body, which implementers can use as-is
or override:

```fig
trait Shape {
    fn area(self) -> float;

    fn describe(self) -> String {
        "a shape" // default — implementers don't have to write this one
    }
}
```

## Traits as types

A trait name can be used directly wherever a type is expected, meaning "any
value that implements this trait" — as a function parameter type, a return
type, or a variable's type annotation:

```fig
fn print_area(shape: Shape) {
    print(shape.area());
}

print_area(Circle { radius: 2.0 }); // any Shape-implementing value works
```

Rust requires writing `&dyn Shape` or `Box<dyn Shape>` for this, because a
trait-typed value in Rust needs an explicit pointer indirection (the type
itself is unsized) and an explicit opt-in to dynamic dispatch via `dyn`.
Since every non-primitive value in fig is already a runtime-managed
reference (see [The Value Model](../design/values-and-mutation.md)), a
trait-typed value works exactly the same way any other reference-typed
value does — no `dyn`, no `Box`, no separate sized/unsized distinction. See
[Differences from Rust](../design/differences-from-rust.md).

This justification is specifically about *reference* types, and doesn't
yet have a settled answer for a bare trait-typed position holding a
*primitive* directly (`let x: Add = 5;`, or a heterogeneous `[Add]`
containing an `int`) — unlike the [generic, statically-resolved case](operator-overloading.md#primitive-types-implement-these-traits-intrinsically),
which sidesteps this entirely since it never needs a runtime representation
for `T`. Not a case fig has a story for yet; flagged here rather than
papered over.

## Trait bounds on generics

The more common use of a trait is constraining a generic type parameter —
see [Generics](generics.md) for the full syntax:

```fig
fn largest<T: PartialOrd>(items: [T]) -> T {
    // ...
}
```

## Associated types

A trait can declare a type that each implementation fills in, useful when a
trait's methods need to refer to a type that varies by implementer:

```fig
trait Container {
    type Item;

    fn get(self, index: int) -> Self::Item;
}

impl Container for IntList {
    type Item = int;

    fn get(self, index: int) -> int {
        self.values[index]
    }
}
```

## Supertraits

A trait can require that implementers also implement another trait, using
the same `:` syntax as a generic bound:

```fig
trait Drawable {
    fn draw(self);
}

trait Shape: Drawable {
    fn area(self) -> float;
}

fn render(shape: Shape) {
    shape.draw();      // ok — every Shape is also Drawable
    print(shape.area());
}
```

## Operator overloading

Operators like `+`, `==`, and `<` are implemented for custom types via
traits — see [Operator Overloading](operator-overloading.md).

## What's *not* here

fig has no equivalent of Rust's `unsafe trait`, no `?Sized`/`Sized` bounds,
and no `dyn`-related object-safety rules — all consequences of removing the
ownership/sizedness machinery those exist to support. See
[Differences from Rust](../design/differences-from-rust.md).
