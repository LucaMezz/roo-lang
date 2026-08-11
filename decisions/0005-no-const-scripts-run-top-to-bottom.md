# 0005. No separate `const`; fig files execute top to bottom like a script

**Status:** Accepted

## Context

With `let` already immutable by default, is `const` actually doing
anything `let` (no `mut`) doesn't? The differences originally documented
were: `const` requires a compile-time-evaluable initializer, requires an
explicit type annotation, and (in Rust) is valid at module scope while
`let` isn't.

Two of those three don't hold up for fig specifically:

- The **compile-time-evaluable guarantee** backs Rust features fig has
  already cut: fixed-size array lengths (fig's arrays are always
  dynamically sized), `static` initialization ordering (fig has no
  `static`), and const generics (not part of fig). Without a use for the
  guarantee, it isn't earning a second binding form.
- The **module-scope restriction** assumed Rust's model, where only items
  are legal outside a function body. That assumption needed checking
  against how fig scripts actually execute.

## Decision

fig has no `const` keyword (kept reserved for possible future use, not
freed up — see `book/src/lexical/identifiers-and-keywords.md`). An
ordinary, immutable, module-level `let` covers the need, with
`SCREAMING_SNAKE_CASE` remaining a pure naming convention, not a
compiler-enforced one.

This forced a second, more foundational confirmation: **fig files execute
top to bottom, like a Lua/Luau chunk, with no `fn main` entry point.**
`let` therefore works identically at file scope as anywhere else — this
is what actually makes `let` a full substitute for `const`'s module-scope
capability.

## Rationale

Luau itself has no separate `const` declaration at all — `local x <const>
= value` is an attribute on an ordinary local, not a distinct binding
form. That's real precedent from the exact language fig models its
gradual typing on, and it lines up with the same conclusion fig's own
removed-use-cases analysis reached independently.

The top-to-bottom execution model has a real, initially-undocumented
consequence: `return` and `?` are only meaningful inside a real function
(by their own definitions), so any program that wants either has to wrap
its logic in an explicit function and call it manually as the last line
(`fn run() { ... } run();`). This was confirmed to be *intentional*, not
a gap — see [0015](0015-trailing-expression-rule-scope.md) for the
related mechanics of `return`.

See: `book/src/bindings/variables.md` ("Module-level bindings" and
"Structuring anything larger than a few lines"),
`book/src/design/differences-from-rust.md`.
