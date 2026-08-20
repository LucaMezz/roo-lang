# roo-lang

An embeddable, statically-typed scripting language with a 
Hindley-Milner-like type system written in
Rust. Roo is heavily inspired by Rust, but aims to be more light-weight. 
In particular, Roo borrows a lot of Rust's syntax, style, and
struct-and-trait model. It leaves behind a lot of low-level concepts such
as systems programming, memory & resource management, macros, etc. It 
includes more powerful type inference.

This is a learning project. I don't claim to be an expert in language
design or implementation. I simply have a deep interest in both of these
areas, and so I decided to create this language and learn as I go. I 
hope this project will improve my understanding of all programming 
languages that I work with, and that it will improve my overall 
problem-solving skills.

## Why

This particular project started when I was implementing a small game
engine written in Rust. I wanted to include a scripting component which
would allow me to easily create new functionality for a game. I wanted
something that was small, and where I didn't have to worry about 
low-level concepts, as performance was not so much of a concern.
I also enjoy working with traits, structs and enums, so I wanted it to
follow that kind of model.

I concidered existing languages such as [Rust](https://www.rust-lang.org/),
[Rhai](https://rhai.rs/), and [Rune](https://rune-rs.github.io/).
Clearly Rust was off the table, considering all the complexities of
lifetimes, ownership, borrowing, references, etc. It wouldn't be easy
to iterate on game features. Rhai and Rune were both good options,
they could both be embedded into Rust. However, Rhai was lacking in
features I wanted, such as traits, structs, first-class functions,
garbage collection, and more. Rune was a lot closer to what I was
after. However, traits aren't exactly use-facing, and it had an entire
macro system , which I didn't really need. At this point, I also decided
that I wanted to be able to annotate types, which would eliminate a 
large class of runtime errors. So Rune was also off the table. 

This is when I decided that I would try creating my own language. I
have wanted to begin working on a project like this for a while, so
I felt that it was a good opportunity to.

## Status

The lexer and parser have been fully implemented with the help of the
external crates `logos` for lexing and `chumsky` for parser combinators.
The type checker and inference engine is a work-in-progress. However,
substantial progress has been made on it. Bytecode generation, the
bytecode VM, and the embedding API have all not yet been started, but
are planned.

- [x] Lexer
- [x] Parser
- [ ] Type checker (in progress)
- [ ] Codegen
- [ ] Bytecode VM
- [ ] Embedding API

I do plan on eventually re-writing the lexer and parser from scratch
rather than using external crates, except I wanted to get to the
typechecker and back-end of the pipeline quicker, since I have
written lexers and parsers from scratch in the past, but have never
dealt with type systems or writing a VM.

## Structure

A Cargo workspace. Every stage of the pipeline has a crate. In addition
to those, there are several other crates which different stages of the
pipeline may depend on.

- [`lexer`](crates/lexer) -- turns roo source text into tokens using `logos`
- [`parser`](crates/parser) -- parses tokens into an AST, using `chumsky`'s parser combinators
- [`ast`](crates/ast) -- the abstract syntax tree roo source parses into
- [`typecheck`](crates/typecheck) -- the type checker; performs type inference and type checking.
- [`unify`](crates/unify) -- a general-purpose, reusable unification engine for solving systems of type equations
- [`diagnostics`](crates/diagnostics) -- locale-agnostic, error-code-tagged diagnostics rendered through Fluent
- [`diagnostics-derive`](crates/diagnostics-derive) -- proc-macro support for `diagnostics`
- [`walkable-derive`](crates/walkable-derive) -- proc-macro support for AST traversal, used by `ast`
- [`cli`](crates/cli) -- roo's reference CLI; lexes, parses, and type checks a `.roo` file
- [`lsp`](crates/lsp) -- roo's language server; diagnostics over stdio for editor integration


## Documentation

The complete documentation for the language has still not yet been started.
However, I plan on writing documentation for the entire language in
[The Roo Programming Language](book), an [mdBook](https://github.com/rust-lang/mdBook) built from
[`book/src`](book/src). It will the language's syntax and semantics.

```sh
cargo install mdbook
mdbook build book --open
```

## License

Licensed under the [MIT license](LICENSE).
