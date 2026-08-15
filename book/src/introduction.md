# Introduction

fig is an embeddable, gradually-typed scripting language. Syntactically and
semantically, it is a subset of [Rust](https://www.rust-lang.org/): if you
know Rust, you already know most of fig, and almost everything that is legal
fig source is legal to *read* as if it were Rust.

fig keeps the parts of Rust that make programs easy to reason about —
expression-oriented syntax, `let` bindings, `match`, an `enum`/`struct` data
model, traits, generics — and drops the parts of Rust that exist to give a
*systems* language precise, unsafe-free control over memory and hardware.
There is no borrow checker, no ownership, no explicit references, no
lifetimes, no `unsafe`, no raw pointers, no manual memory management or
allocators, no FFI, and no macros. A full account of what was left out, and
why, is in [Differences from Rust](design/differences-from-rust.md).

On top of that smaller core, fig adds one thing Rust does not have: type
annotations are optional almost everywhere. Write one and the compiler
checks it; leave it off and fig **infers** the strongest type it can —
which, for a whole function left completely unannotated, can mean
inferring generic type parameters it was never told about, not just a
single concrete type. This is stronger than the gradual typing
[Luau](https://luau.org/) (Roblox's dialect of Lua) and TypeScript are
usually compared to, which fall back to dynamic, run-time-checked typing
wherever nothing's annotated — fig only falls back that far in the
narrower set of places inference genuinely can't reach yet, or where the
explicit `any` type asks for dynamic behavior on purpose. See
[Type Inference](types/inference.md) and
[Gradual Typing](types/gradual-typing.md) for the full rules, and
[Design: Philosophy](design/philosophy.md#gradual-typing-vs-strong-inference)
for why fig ended up here. Either way, a script can start as loose,
ordinary-looking code with no type annotations at all and grow them
incrementally, in exactly the places where they earn their keep.

## Who this book is for

This book is the language specification and reference guide for fig. It
documents the *syntax and semantics of the language itself* — every kind of
statement, expression, declaration, and type that fig understands. It does
**not** document a standard library, because fig does not have one yet. Where
a language feature (like the `?` operator, or `for` loops) depends on a
standard-library concept that hasn't been designed, this book describes the
language-level contract fig guarantees and leaves the concrete library
design as future work.

## Project status

fig is early. As of this writing, the
[fig-lang](https://github.com/LucaMezz/fig-lang) repository has a working
lexer, a parser covering nearly all of the syntax this book documents,
and a type checker that implements the inference/gradual-typing model
described here — including generalizing untyped functions and type
aliases into real generic types, the way [Type
Inference](types/inference.md) describes. There is no interpreter,
compiler, or standard library yet. This book documents the *intended*
design of the language — the target this implementation is being built
towards — not a fully shipped, battle-tested feature set, and some
corners (generic `struct`/`enum` types, traits, recursive-function
generalization) are still ahead of what's actually implemented. Expect it
to change as the implementation reveals what does and doesn't work.

fig is being developed alongside
[fig-engine](https://github.com/LucaMezz/fig-engine), a game engine, which it
is intended to eventually serve as the embedded scripting language for
(replacing Luau/`mlua`).

## How to read this book

The chapters are ordered roughly the way you'd introduce the language to
someone who already knows how to program:

- **Design** explains the ideas that hold the rest of the language together
  — the philosophy behind mixing Rust's syntax with gradual typing, and the
  value/mutation model that replaces ownership and borrowing.
- **Lexical Structure**, **Types**, **Variables and Bindings**, and
  **Expressions and Operators** cover the small building blocks.
- **Control Flow**, **Functions and Closures**, **Custom Data Types**, and
  **Abstraction** (traits and generics) cover the constructs you compose
  those building blocks with.
- **Program Organization** and **Error Handling** cover how larger programs
  are structured.
- The **Appendix** collects quick-reference material: keywords, operator
  precedence, and a summary grammar.

Every syntax construct in this book is shown with a runnable-looking code
sample. Code blocks are labeled `fig`, not `rust` — the syntax is very close,
but treat every example as fig source, not Rust source, since small
semantic differences (noted inline) do apply.
