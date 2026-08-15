# Type System Overview

fig's type system has three ingredients: a small set of built-in types, the
custom types you define (`struct`, `enum`), and gradual typing, which
determines when any of it is actually enforced.

## Kinds of types

- **Primitive (value) types**: [`bool`, `int`, `float`, `char`](primitives.md).
  Copied on assignment and passing.
- **Built-in reference types**: [`String`](strings-and-chars.md) and
  [arrays and tuples](arrays-and-tuples.md). Shared on assignment and
  passing, like every custom type — see
  [The Value Model](../design/values-and-mutation.md).
- **Custom types**: [`struct`s and `enum`s](../data-types/structs.md) you
  define. Also reference types.
- **Function/closure types**: written `Fn(ParamTypes) -> ReturnType` — see
  [Closures](../functions/closures.md).
- **The unit type**, `()`: the type of expressions evaluated only for their
  side effects. Write `-> ()` on a function to statically guarantee it
  returns nothing regardless of later edits to the body; *omitting* the
  return type annotation instead infers it from the body, which comes out
  to exactly `()` whenever the body has no trailing expression — see
  [Type Inference](inference.md) and
  [Functions: Return type](../functions/functions.md#return-type).
- **The `any` type**: written explicitly, it means "dynamically typed
  here," overriding whatever fig would otherwise have inferred — see
  [Gradual Typing: The explicit `any` type](gradual-typing.md#the-explicit-any-type).
  Not a keyword, just a builtin type name like `int` or `String`.
- **Trait types**: a trait name used where a type is expected means "any
  value implementing this trait" — see [Traits](../abstraction/traits.md).
- **Generic type parameters**: `T`, `U`, ... introduced by `<...>` on a
  function, struct, enum, or trait — see [Generics](../abstraction/generics.md).

## Where type annotations go

The grammar for a type annotation is the same in every position: a colon
followed by a type.

```fig
let x: int = 5;                          // variable binding
fn add(a: int, b: int) -> int { a + b }  // parameters and return type
struct Point { x: float, y: float }      // struct fields
fn identity<T>(x: T) -> T { x }          // generic parameters
```

Every one of these annotations is **optional** — omitting one falls back
to [type inference](inference.md) in most positions now, including a
function's entire signature (parameters, return type, and even its own
`<T>` list), not just a `let`'s initializer. Only struct/enum fields, a
`let` with no initializer, and anywhere `any` is written explicitly still
fall back to dynamic typing. That boundary — and why it's smaller than
you might expect from a gradually-typed language — is the subject of the
next two chapters, [Type Inference](inference.md) and
[Gradual Typing](gradual-typing.md).

## No `null`, no implicit conversions

Like Rust, fig has no `null`/`nil` value that inhabits every type. Absence
of a value is represented with an explicit type (an `Option`-shaped enum, in
the standard library sense — see [Error Handling](../errors/error-handling.md)),
so "forgetting" to handle the absent case is a type error, not a runtime
crash waiting to happen.

Also like Rust, fig performs no implicit numeric or boolean coercions: an
`int` is never silently used where a `float` is expected, and non-boolean
values are never silently treated as truthy/falsy in an `if` condition.
Conversions are always explicit, via [`as`](casting.md).
