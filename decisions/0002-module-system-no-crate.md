# 0002. A Rust-like module system, but no crate-level compilation unit

**Status:** Accepted

## Context

Two separate questions came up in sequence:

1. Rust's `crate` — the compilation unit a module tree lives inside, with
   its own `crate::` path root and `pub(crate)` visibility tier — is
   specifically Cargo/Rust-ecosystem vocabulary. Should fig keep it, rename
   it, or drop it?
2. Should fig have a module system (`mod`/`use`/`pub`) at all, given
   Lua/Luau — the languages fig is closest to and currently transpiles to
   — have no module system in the language itself? Lua's compilation unit
   is a "chunk"; modules are just a `require`-based convention returning a
   table, with no privacy system in the language.

An initial instinct was to eliminate modules from fig entirely, mirroring
Lua/Luau, and let a script's top-level `pub` items play the role Lua's
`return { ... }` table plays. That direction was reversed: modules do
exist, and should be Rust-like, specifically because Rust code (fig-engine)
needs to expose modules, functions, types, and methods to scripts, and a
Rust-shaped module/trait/impl system is what makes that mirroring possible
(see [0004](0004-ambient-modules.md)).

Separately, on naming: renaming `crate` to `package` was considered and
rejected — in real Rust/Cargo, "package" and "crate" are already two
distinct, differently-scoped terms (a package can contain several
crates), so reusing "package" for fig's crate-equivalent would just
relocate the confusion rather than resolve it.

## Decision

fig has a full, Rust-shaped module system: `mod`, `use` (including `pub
use`, see [0011](0011-pub-use-reexport.md)), `pub`/`pub(super)`,
`self::`/`super::` paths. There is no `crate` keyword, no `crate::` path
root, and no `pub(crate)` visibility. An unqualified path resolves from
the root of the module tree by default (mirroring modern Rust's own rule
for a bare path that isn't a local item or `use`).

## Rationale

Removing `crate` is a pure removal, not a rename — the same shape as
dropping `unsafe` or lifetimes, requiring no new vocabulary. It's
justified because fig's own module tree, once fully resolved, *is* the
whole program: fig's `mod`/`use` are resolved entirely at compile time by
the transpiler (see [0003](0003-transpile-to-luau.md)), so there's no
second, coarser compilation unit above "module" for `crate` to usefully
name — no notion of "this compiled unit" vs. "a separate crate depending
on it" exists at all.

Keeping the module system itself (rather than going full Lua-chunk) was
justified by [0004](0004-ambient-modules.md)'s needs: mirroring a Rust API
surface into fig only works cleanly if fig's `mod`/`struct`/`trait`/`impl`
are shaped like Rust's.

See: `book/src/modules/modules.md`,
`book/src/design/differences-from-rust.md`.
