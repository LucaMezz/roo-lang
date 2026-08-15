# Generics

A function, struct, enum, or trait can be parameterized over one or more
types, written as `<T>` (or `<T, U, ...>`) after its name, exactly as in
Rust.

## Generic functions

```roo
fn first<T>(items: [T]) -> T {
    items[0]
}

first([1, 2, 3]);           // T = int
first(["a", "b", "c"]);     // T = String
```

Type arguments are usually inferred at the call site (see
[Type Inference](../types/inference.md)) and can also be given explicitly:

```roo
let x = first::<int>([1, 2, 3]);
```

## Generic structs and enums

```roo
struct Pair<T> {
    first: T,
    second: T,
}

let p = Pair { first: 1, second: 2 };       // Pair<int>
let q = Pair { first: "a", second: "b" };   // Pair<String>
```

```roo
enum Either<L, R> {
    Left(L),
    Right(R),
}
```

## Trait bounds

A bare generic parameter (`<T>`) can be any type at all, including one with
no methods usable on it. A **trait bound** restricts the parameter to types
implementing a given trait, which is what makes it possible to call that
trait's methods on values of type `T`:

```roo
fn largest<T: PartialOrd>(items: [T]) -> T {
    let best = items[0];
    for item in items {
        if item > best { // requires PartialOrd
            best = item;
        }
    }
    best
}
```

Multiple bounds combine with `+`:

```roo
fn print_and_compare<T: Display + PartialOrd>(a: T, b: T) {
    print(a);
    print(b);
    print(a < b);
}
```

## `where` clauses

For functions with several bounded parameters, a `where` clause after the
signature is equivalent to inline bounds but easier to read:

```roo
fn process<T, U>(a: T, b: U) -> bool
where
    T: PartialOrd,
    U: Display,
{
    // ...
}
```

## Generic `impl` blocks

Methods can be defined generically, and can also be restricted to specific
type arguments:

```roo
impl<T> Pair<T> {
    fn new(first: T, second: T) -> Pair<T> {
        Pair { first, second }
    }
}

impl<T: PartialOrd> Pair<T> {
    fn largest(self) -> T {
        if self.first > self.second { self.first } else { self.second }
    }
}
```

## Generic traits

A trait itself can be generic:

```roo
trait Converter<T> {
    fn convert(self) -> T;
}
```

## Default type parameters

A generic parameter can declare a default, filled in whenever it's left
unspecified at the `impl`/use site:

```roo
trait Converter<T = String> {
    fn convert(self) -> T;
}

impl Converter for Point {       // T defaults to String here
    fn convert(self) -> String {
        // ...
    }
}

impl Converter<int> for Point {  // explicit T overrides the default
    fn convert(self) -> int {
        // ...
    }
}
```

This exists mainly so a generic trait can have one common case that reads
exactly as tersely as a non-generic one, while still allowing less common
instantiations to be spelled out explicitly. The main place this matters
is [operator overloading](operator-overloading.md#the-right-hand-side-isnt-always-self),
where the right-hand-side type of an operator is usually — but not always
— the same as the left-hand side.

## How generics interact with gradual typing and inference

Generics are a purely static-typing feature: a generic parameter with no
bound (`<T>`) still means "some specific, statically-tracked type, the same
for every use of `T` in this signature," not "dynamically typed." If you
want a genuinely dynamically-typed parameter, annotate it `any` rather
than reaching for a generic — see [Gradual Typing](../types/gradual-typing.md).
The two features solve different problems: generics let one signature
work polymorphically across many *static* types; gradual typing lets a
binding opt out of static typing altogether, on purpose.

Omitting a function's type annotations entirely does **not** default to
dynamic typing either, and often produces a generic function anyway —
just an *inferred* one, arrived at without writing `<T>` yourself. See
[Type Inference](../types/inference.md) for the full story on when that
happens and why the result is exactly the same kind of generic function
as one you declare explicitly.
