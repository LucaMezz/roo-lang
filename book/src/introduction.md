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

On top of that smaller core, fig adds one thing Rust does not have: **gradual
typing**, in the same sense that [Luau](https://luau.org/) (Roblox's dialect
of Lua) and TypeScript use the term. Type annotations are optional almost
everywhere. Write them and the compiler checks them; leave them off and the
value is treated as dynamically typed, the way an untyped `any` is in Luau or
TypeScript. This lets a script start as loose, ordinary scripting-language
code and grow type annotations incrementally, in exactly the places where
they earn their keep.

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

fig is early. As of this writing, the only implemented piece is the
`lexer` crate in the [fig-lang](https://github.com/LucaMezz/fig-lang)
repository, which turns source text into tokens. There is no parser, type
checker, or interpreter/compiler yet. This book documents the *intended*
design of the language — the target this implementation is being built
towards — not a shipped, battle-tested feature set. Expect it to change as
the implementation reveals what does and doesn't work.

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
