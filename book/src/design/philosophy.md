# Philosophy

fig is guided by a small number of governing ideas. When a piece of Rust
syntax doesn't obviously fall into "keep" or "cut," these are the principles
that settle it.

## Rust's surface, minus its systems-programming payload

Rust's syntax is, on the whole, an excellent design: expression-oriented,
explicit about control flow, with a data model (`struct`/`enum`/`match`)
that scales from small scripts to large programs. Most of what makes Rust
*pleasant* to read and write has nothing to do with memory safety — it's the
`if`/`match` as expressions, the exhaustiveness checking, the trait system,
the absence of implicit conversions and null.

What makes Rust *hard to learn* and slow to write quick, throwaway code is
almost entirely the part of the language that exists to manage memory
without a garbage collector: ownership, borrowing, lifetimes, `unsafe`. fig
keeps the former and discards the latter. Underneath fig's implementation
there will be a runtime that manages memory automatically (garbage
collection or an equivalent), so none of the machinery Rust needs to avoid
one is necessary here. See [Differences from Rust](differences-from-rust.md)
for the full list of what that removes.

## Where fig started, and where it pivoted to

fig's starting brief was simple: a scripting language with Rust's surface
syntax, minus the systems-programming payload described above, plus
*gradual* typing — optional type annotations, in the same sense Luau (for
Roblox) and TypeScript (for JavaScript) use the term. The idea was that
you could write loose, untyped, ordinary scripting-language code to get
something working quickly, the same way you would in plain Lua or
JavaScript, and add `: Type` annotations incrementally, in exactly the
places where locking something down actually earns its keep — without
ever needing to fully commit either way for an entire file.

That's still true today, but the mechanism behind "few or no
annotations stays easy to work with" changed partway through the
language's design. The original plan mirrored Luau and TypeScript
exactly: leave a type off, and that position falls back to being
*dynamically typed*, checked at run time instead of compile time — real
gradual typing, the same trade Luau and TypeScript make. What fig has now
instead is much stronger: leaving a type off asks the checker to *infer*
the strongest static type it can — including inferring an entire
function's signature, generic type parameters and all — before it would
ever fall back to dynamic typing. See
[Gradual typing vs strong inference](#gradual-typing-vs-strong-inference)
below for what motivated the change, and
[Type Inference](../types/inference.md) for the full mechanics.

## Gradual typing vs strong inference

The beginner-friendliness and fast-prototyping benefits gradual typing is
supposed to buy you — not needing to think about types if you don't want
to, not having to write annotations everywhere just to get something
running, being able to sketch first and harden later — don't actually
require *dynamic* typing to get. They just require *not being forced to
write types out by hand*. Real type inference gets you the same
ergonomic win, for the positions it can reach, without giving up static
checking to do it:

```fig
fn identity(x) {
    x
}
```

Written with no annotations at all, `identity` isn't dynamically typed —
fig infers it to exactly the type you'd get by writing
`fn identity<T>(x: T) -> T` yourself, generic parameter included. Calling
it at two different types in the same program (`identity(5)` and
`identity("hi")`) is completely ordinary, the same freedom untyped code
has in Lua — but `let n: int = identity("oops");` is still a real,
static, compile-time type error, not something that waits to blow up at
run time the way it would in an actually dynamically-typed language.

That's the shape of the pivot: instead of "no annotation means dynamic,
checked at run time," fig's rule became "no annotation means inferred,
checked at compile time — dynamic typing is what's left over for the
positions inference genuinely can't reach, or where `any` asks for it
explicitly." Practically, that turns a class of what would have been
runtime errors (in the original all-the-way-to-dynamic design, and in
Luau/TypeScript's actual gradual typing) into compile-time errors
instead, without costing back any of the "don't make me write out types
I don't care about" ergonomics that motivated wanting gradual typing to
begin with — you get most of the benefit ordinarily attributed to
dynamic typing, and most of the benefit of static typing, at the same
time, in the same unannotated code.

`any` is still a real part of the language — see
[Gradual Typing](../types/gradual-typing.md) — and remains the way to ask
for genuinely dynamic, unchecked-until-runtime behavior on purpose, even
somewhere inference would otherwise happily produce a concrete or generic
type. Whether fig keeps a way to opt an entire binding out of static
checking indefinitely, the way `any` does today, is still an open
question as the language's design settles — but as things stand right
now, `any` is that escape hatch, and it's a supported, intentional
feature, not a stopgap.

This is also why fig is a *scripting* language and not "Rust with a
garbage collector": you can still write a 15-line script with no type
annotations at all and have it feel as unburdened as a dynamically-typed
one to write — you just get compile-time errors for the mistakes a
dynamically-typed script would only have caught by actually hitting them
at run time. See [Gradual Typing](../types/gradual-typing.md) and
[Type Inference](../types/inference.md) for the full rules.

## A subset, not a variant

Where fig keeps a piece of Rust syntax, it tries to keep it *exactly as
Rust defines it* — same keyword, same shape, same meaning — rather than
inventing a similar-but-different alternative. The goal is that a Rust
programmer's intuition transfers directly, and that fig source reads as
"Rust, with some things missing," never as "a language that looks like Rust
but secretly isn't."

Where fig *must* diverge, because the Rust feature exists specifically to
serve ownership/borrowing/memory layout (for example, `self` vs. `&self` vs.
`&mut self`, or fixed-width integer types), the divergence is called out
explicitly wherever it comes up, and summarized in
[Differences from Rust](differences-from-rust.md).

## Small core, standard library does the rest

Many things a working language needs — collections beyond arrays and
tuples, I/O, string formatting, `Option`/`Result` as concrete types, math
functions — are standard-library concerns, not language syntax. fig's
standard library has not been designed yet, and this book deliberately does
not try to design it. Where language syntax has an unavoidable dependency on
a library concept (the `?` operator needs *some* error type; `for` loops
need *some* iteration protocol), this book specifies the contract the
language guarantees and defers the concrete types to the standard library.
