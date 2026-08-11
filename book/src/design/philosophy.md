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

## Gradual typing, not optional typing bolted on

fig's type system is designed the way Luau's and TypeScript's are: every
binding, parameter, and return type *can* carry a type annotation, and
*none* of them are required to. An annotation is a promise the checker
verifies; the absence of one is not an error — fig either infers a type
from context when it unambiguously can (a `let` with an initializer,
mainly) or, when it can't, treats the value as dynamically typed, checked
at run time, like a value in plain Lua, Python, or JavaScript. Either way,
writing `any` explicitly always means the latter, on demand.

This is a deliberate difference from Rust, where static types are mandatory
everywhere (even when inferred). It is also the reason fig is a *scripting*
language and not "Rust with a garbage collector": you can write a
15-line script with no type annotations at all, and it behaves like a
dynamically-typed script, right up until you start adding `: Type`
annotations to lock parts of it down. See
[Gradual Typing](../types/gradual-typing.md) for the full rules.

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
