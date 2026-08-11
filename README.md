# fig-lang

An embeddable, gradually-typed scripting language written from scratch in
Rust — a subset of Rust itself (no lifetimes or other low-level machinery),
with optional type annotations: anything left untyped is skipped by type
checking, the same way Luau or TypeScript treat `any`.

This is a learning project, developed alongside
[fig-engine](https://github.com/LucaMezz/fig-engine), which it's intended to
eventually replace Luau/`mlua` as the scripting language for. Expect the API
(and the language itself) to change as it grows.

## Status

Early scaffolding — no lexer, parser, type checker, or codegen yet.

## Structure

A Cargo workspace, one crate per stage of the pipeline (mirroring
fig-engine's own `crates/*` layout):

- [`crates/lexer`](crates/lexer) — turns source text into tokens. The only
  crate that exists so far.

## Documentation

The language itself — every kind of statement, expression, declaration, and
type fig understands — is specified in [The fig Programming
Language](book), an [mdBook](https://github.com/rust-lang/mdBook) built from
[`book/src`](book/src). It documents the language's intended design, ahead
of the implementation. Build it locally with:

```sh
cargo install mdbook
mdbook build book --open
```

## License

Licensed under the [MIT license](LICENSE).
