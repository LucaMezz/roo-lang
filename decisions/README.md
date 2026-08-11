# Design decisions

An [ADR](https://en.wikipedia.org/wiki/Architectural_decision)-style log of
fig's real design decisions — the ones with actual alternatives weighed
and a reason for landing where they landed, not just "what the syntax is"
(the book already covers that). Each file: the question that came up, what
was decided, and why — including alternatives that were considered and
rejected, where that matters.

Numbered in the order the decisions were made. Nothing here is
necessarily final; a later decision can supersede an earlier one, and
should say so when it does.

| # | Decision |
|---|---|
| [0001](0001-value-model-reference-semantics.md) | Compound types are reference-typed, primitives are value-typed; `mut` gates both reassignment and mutation-through, uniformly everywhere |
| [0002](0002-module-system-no-crate.md) | fig keeps a Rust-like module system (`mod`/`use`/`pub`) but has no crate-level compilation unit above modules |
| [0003](0003-transpile-to-luau.md) | fig's implementation strategy is a source-to-source transpiler targeting Luau |
| [0004](0004-ambient-modules.md) | Host-provided functionality is expressed as ordinary fig modules with bodyless function signatures |
| [0005](0005-no-const-scripts-run-top-to-bottom.md) | No separate `const` — module-level `let` covers it; fig files run top to bottom like a script, no `fn main` |
| [0006](0006-gradual-typing-inference-and-any.md) | Refined gradual-typing rule: infer from a local source when one exists, else dynamic; `any` is a real type that overrides inference |
| [0007](0007-operator-trait-shape-rhs-output.md) | Operator traits take `Rhs` (defaulting to `Self`) and an `Output` associated type, plus default generic type parameters to support it |
| [0008](0008-pattern-level-mut.md) | `mut` can bind to any individual identifier inside a pattern; removed the redundant separate `mut` slots on `let`/parameters |
| [0009](0009-hoisting-and-closure-capturing.md) | Items are hoisted, `let` bindings aren't — which is why closures capture their environment and plain `fn` items don't |
| [0010](0010-tostring-intrinsic.md) | Primitive-to-`String` conversion goes through a `ToString` trait, not `Into`/`From` |
| [0011](0011-pub-use-reexport.md) | `pub use` re-exports an imported item through the current module's own path |
| [0012](0012-tuple-struct-field-visibility.md) | Each positional field in a tuple struct has its own `pub`, mirroring named fields |
| [0013](0013-loop-label-disambiguation.md) | `break`/`continue`'s identifier is always a label if one's in scope, never a value — no escape-hatch syntax |
| [0014](0014-dynamic-return-fn-boundary.md) | A dynamically-typed-return function can be stored where a concrete `Fn(...) -> T` is expected; checked at the call |
| [0015](0015-trailing-expression-rule-scope.md) | The trailing-expression rule only applies when a block completes normally — a diverging statement preempts it |
| [0016](0016-labeled-loop-codegen-luau.md) | Compilation strategy for labeled `break`/`continue` targeting Luau, which has no `goto`/labels |
