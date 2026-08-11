# 0004. Host-provided functionality is expressed as ambient modules

**Status:** Accepted

## Context

fig-engine (Rust) needs to expose native modules, functions, types, and
methods to fig scripts. fig has deliberately removed all of Rust's FFI
machinery (`extern`, `#[repr(C)]`, `#[no_mangle]`) as low-level/systems
concerns that don't fit a high-level scripting language. The question was
how a script author writes code against Rust-provided functionality at
all, given fig transpiles to Luau ([0003](0003-transpile-to-luau.md)) and
the actual binding into the running Luau VM happens at the host level,
entirely outside fig-the-language.

## Decision

A function declared with no body — `;` instead of a block, the same
allowance trait method declarations already have — means "the host
embedding fig provides this." A module built out of such signatures is an
**ambient module**, used with completely ordinary `use`/path/call syntax,
indistinguishable at the call site from a fig-authored module. Nothing
marks the call site as foreign; the missing body at the *declaration* is
the only signal.

At the fig-engine end (see [0003](0003-transpile-to-luau.md)): fig-engine
registers the real implementation into the Luau VM's globals before a
transpiled script runs — one global table per top-level ambient module —
and the transpiler emits a plain global lookup for each ambient call
(`engine::Vector3::new(...)` → `Vector3.new(...)`), not a `require()`.
This mirrors how Lua/Luau hosts conventionally expose engine functionality
(Roblox's `Vector3`, `game`, `workspace`) as ambient globals rather than
`require`-able modules.

## Rationale

This is fig's replacement for `extern` blocks, without any of the FFI
ceremony — ambient modules describe only fig-level type signatures, no
calling convention or memory layout, which is exactly the boundary fig
wants (host-provided, still fully gradually-typed, no `unsafe`).

How fig-engine actually authors and keeps these declarations in sync with
the real Rust implementation (hand-written stub files vs. generated from
the real Rust types via a macro/build step) is deliberately left
unspecified — a fig-engine tooling decision, not a fig language one.

See: `book/src/modules/ambient-modules.md`,
`book/src/design/differences-from-rust.md`.
