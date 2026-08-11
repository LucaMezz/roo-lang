# 0003. fig's implementation strategy is a transpiler targeting Luau

**Status:** Accepted (implementation strategy, not language semantics —
see note below)

## Context

fig-lang's README frames it as intended to "eventually replace Luau/`mlua`
as the scripting language" for fig-engine. That phrasing alone doesn't say
*how* fig would run — a standalone bytecode VM/interpreter is a different
project than a source-to-source compiler targeting an existing VM.

## Decision

fig currently compiles by transpiling to Luau source, which runs on a
Luau VM embedded in fig-engine (presumably via `mlua` or similar, given
the README's own framing).

## Rationale

Stated directly during module-system design, as the reason cross-file and
host-interop design has to be grounded in what's actually expressible in
Luau and in how a Rust host embeds a Luau VM (globals, userdata/metatables,
Luau's own `require`) — not an abstract fig VM with its own rules.

This has already shaped two other decisions concretely:
[0004](0004-ambient-modules.md) (ambient modules map to Luau globals
injected before a script runs, not to Luau's `require`) and
[0016](0016-labeled-loop-codegen-luau.md) (labeled `break`/`continue` need
a real desugaring strategy specifically because Luau has no `goto`/labels
of its own).

## Caveat

This is explicitly a "currently" fact about implementation strategy, not
a piece of language semantics — it's recorded here because it's
load-bearing for other decisions, but it should be re-confirmed if it's
been a while, since an early-stage project's implementation strategy can
change without the language design changing.
