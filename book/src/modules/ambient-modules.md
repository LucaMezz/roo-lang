# Ambient Modules

Every function shown so far has had a body. roo also allows a function
signature with **no** body, ending in `;` instead of a block — the same
allowance [trait method declarations](../abstraction/traits.md) already
have:

```roo
mod engine {
    struct Vector3 { x: float, y: float, z: float }

    impl Vector3 {
        fn new(x: float, y: float, z: float) -> Vector3;
        fn length(self) -> float;
    }
}
```

A bodyless function isn't implemented in roo at all — it's a promise that
*something outside this roo program* provides it. A module built entirely
out of such signatures is an **ambient module**: a description of an API
surface the host embedding roo (for [roo-engine](https://github.com/LucaMezz/roo-engine),
code written in Rust) makes available to scripts, given in exactly the same
syntax as a module you'd write yourself.

## Using one

From a script's point of view, there is no difference at all between an
ambient module and a roo-authored one. Same `use`, same paths, same
`::`/`.` call syntax, same static type checking:

```roo
use engine::Vector3;

let position = Vector3::new(1.0, 2.0, 3.0);
print(position.length());
```

Nothing marks this call site as "foreign." There's no `extern`, no FFI
ceremony, no `unsafe` — see
[Differences from Rust](../design/differences-from-rust.md). The absence of
a body is the *only* signal, and it lives at the declaration, not the call
site, exactly the way calling a trait method doesn't look different from
calling one with a default implementation.

## Why this, instead of `extern`

Rust's equivalent tool, `extern "C" { fn foo(...); }`, exists to describe a
C ABI boundary: calling convention, memory layout, raw pointers — all of
the low-level detail roo has no use for (see
[Differences from Rust](../design/differences-from-rust.md)). An ambient
module describes none of that. It's a normal, gradually-typed roo
signature; the type checker treats it exactly like any other module's
public API, and everything about *how* the host actually fulfills it is
outside roo's concern.

## How the host fulfills one

roo doesn't mandate an implementation strategy for this — it's a property
of whatever's embedding roo, not of the language. What follows is how it
works today, for roo-engine specifically, since that's the motivating case
this feature was designed around.

roo currently compiles by transpiling to Luau, run on a Luau VM embedded in
roo-engine. Before a transpiled script runs, roo-engine registers the
implementation behind each ambient module into that VM's globals — one
global table per top-level ambient module, so `engine::Vector3` becomes a
Luau global table named `Vector3`. The transpiler's half of the deal is
just a fixed, mechanical convention: a call like `Vector3::new(1.0, 2.0,
3.0)` is emitted as the Luau expression `Vector3.new(1.0, 2.0, 3.0)` — a
plain global lookup, resolved before the script ever starts running, not a
`require()` call. This mirrors how a Lua/Luau host conventionally exposes
engine functionality as ambient globals (Roblox's `Vector3`, `game`, and
`workspace` all work this way) rather than through Luau's module system.

This is a different resolution story from
[roo's own modules](modules.md), which are resolved entirely at compile
time by the roo→Luau transpiler and never touch the Luau VM's globals or
its `require` at all. Ambient modules are the one
place where "this value comes from outside the compiled program" is true.

## Keeping declarations in sync with Rust

How roo-engine actually produces and maintains these ambient declarations —
hand-written stub files alongside the Rust implementation, or generated
automatically from the real Rust types and `impl` blocks by a macro or
build step — is a roo-engine tooling decision, not part of the roo
language. The language only specifies the shape (a module of bodyless
signatures) and the guarantee (it type-checks and is called exactly like
any other module).
