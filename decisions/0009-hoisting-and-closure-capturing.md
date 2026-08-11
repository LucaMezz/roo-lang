# 0009. Hoisting is why `fn` doesn't capture and closures do

**Status:** Accepted

## Context

Found while writing `examples/dialogue.fig`: a plain `fn` was written that
referenced an outer `let` binding, which turned out to be relying on
undocumented behavior — `closures.md` frames "capturing" as specifically a
closure ability ("a closure is an anonymous function that **can**
capture..."), implying a plain `fn` can't, mirroring Rust's fn-item-vs-
closure split. But that was never stated as an explicit rule, and Rust's
own reason for the split (ownership bookkeeping) doesn't even apply to
fig, which has no ownership. So: is there still a real reason for fig to
keep the split, or should it be reconsidered/removed?

## Decision

Keep the split, but for a fig-native reason, not an inherited Rust one:
**items are hoisted, `let` bindings aren't.**

`fn`/`struct`/`enum`/`trait`/`impl`/`mod` declared inside a block are
visible throughout that whole block, including *before* their own
textual declaration — a nested `fn` can be called from code above it. A
`let` binding, by contrast, only exists from its own line onward.

A hoisted `fn` could therefore be called from code that runs *before* a
`let` it might want to capture — there'd be no well-defined value to use
at that point. A closure isn't hoisted; it's an ordinary expression,
evaluated in normal top-to-bottom execution order, so by the time it
captures anything, that thing already has a value. Hoisted things can't
capture, and things that capture aren't hoisted — that's the actual line
fig draws between `fn` and closures.

## Rationale

This reasoning is fig-specific and holds up independent of Rust's
ownership model, unlike simply inheriting Rust's rule without
re-justifying it. It also gave "hoisting" itself a proper name and
explanation in the book for the first time — it had been described in
passing ("visible... before the point they're declared textually") but
never named or connected to why it matters.

See: `book/src/expressions/blocks.md` ("Nested functions and types, and
hoisting"), `book/src/functions/closures.md` ("Why `fn` and closures
capture differently").
