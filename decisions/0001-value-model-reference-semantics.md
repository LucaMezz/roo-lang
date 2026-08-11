# 0001. Compound types are reference-typed, primitives are value-typed

**Status:** Accepted

## Context

Rust's rules for how values move between variables, function calls, and
struct fields come from ownership and borrowing, enforced by the borrow
checker. fig has neither. Something still has to answer the question Rust
answers with `&`, `&mut`, and `move`: when you assign a value to a new
variable, or pass it to a function, do you get the same data or a copy?

Three real options existed:

- **Reference semantics for compound types**: structs, enums, arrays, and
  strings behave like Lua tables or JS objects — assigning/passing shares
  the same underlying data. Primitives (`bool`/`int`/`float`/`char`)
  remain simple value types.
- **Value semantics everywhere**: assignment and passing always
  (conceptually) copy, even for structs. No aliasing, but a callee can
  never mutate the caller's data without an explicit return.
- **Rust-style move semantics without a borrow checker**: assigning or
  passing a non-`Copy` value moves it, invalidating the old binding — but
  with no compile-time enforcement, so use of a moved-from binding would
  need to be a runtime error.

## Decision

Reference semantics for compound types; value semantics for primitives.

## Rationale

This is how the scripting languages fig is closest in spirit to already
behave (Lua, Luau, JavaScript, Python), including Luau specifically, the
language fig is meant to eventually replace as fig-engine's scripting
language. Move semantics without a borrow checker was rejected because it
recreates Rust's most user-hostile failure mode (use of a moved-from
value) with none of the compile-time safety net that makes it bearable in
Rust. Pure value semantics was rejected because it would make ordinary
mutation-through-a-function-call impossible without an explicit return —
exactly the kind of ceremony fig is trying to avoid by not having
references at all.

A consequence worth stating explicitly: `mut` on a binding (`let`,
function/closure parameter, or `self`) is uniform everywhere — it gates
both reassigning that binding and mutating through it (fields, indices,
or calling a `mut self` method), regardless of whether the binding is a
primitive or a reference type. This was confirmed to actually be uniform
in practice while writing worked examples (`fn use_item(mut health: int,
...)` behaves exactly like `fn bump(mut p: Point)`), not just uniform in
theory.

See: `book/src/design/values-and-mutation.md`,
`book/src/design/philosophy.md`.
