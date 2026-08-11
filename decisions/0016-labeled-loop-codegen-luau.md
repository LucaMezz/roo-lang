# 0016. Labeled `break`/`continue` desugar to flag propagation in Luau

**Status:** Accepted (implementation strategy — not reflected in the
language book, which documents fig's syntax/semantics, not its codegen)

## Context

fig transpiles to Luau ([0003](0003-transpile-to-luau.md)). Luau has
`break` and `continue` (added specifically so scripts don't need `goto`),
but both only ever affect the *innermost* loop — and Luau has no
`goto`/labels at all. (Confirmed against Luau's own compatibility
documentation: "goto statement ❌ — this complicates the compiler, makes
control flow unstructured and doesn't address a significant need.") fig's
own labeled loops (`outer: for ... { break outer; }`) therefore have
nothing to lower to directly; the transpiler needs a real desugaring
strategy.

Considered:

- **`goto`-based desugaring** — not available at all, Luau has none.
- **Flag propagation**: a synthetic boolean per referenced label, set
  right before an ordinary `break` out of the innermost loop, checked
  (and re-`break`/`continue`d) at each enclosing loop level until the
  signal reaches its target.
- **Wrap each loop body in a function, use `return` as a sentinel-
  carrying "jump."**
- **`pcall`/`error`-based non-local exit** (throw a sentinel, catch it at
  the target loop).

## Decision

Flag propagation.

## Rationale

`break`/`continue` are always structured, hierarchical jumps — flag
propagation covers everything they can express, generates one boolean
check per intermediate loop level (only for labels actually referenced
from a nested scope, resolved statically since fig labels are lexically
scoped), and has no runtime cost beyond that. The function-wrapping
approach pays a real function-call cost per iteration, which matters more
for fig than for a generic transpiler given the target audience (game-
engine hot loops). The `pcall`/`error` approach is solving a more general
problem (arbitrary non-local exit) than labeled break/continue actually
needs, so it buys nothing here over the cheaper structured option.

This decision is compiler-internal — it constrains what the fig→Luau
transpiler must generate, not anything a script author writes or sees,
so it isn't reflected in the language book itself.
