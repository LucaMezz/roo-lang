# 0015. The trailing-expression rule only applies when a block finishes normally

**Status:** Accepted (clarification/correction, not a new alternative
weighed)

## Context

The trailing-expression rule ("a block's value is its final expression if
there's no trailing `;`, else `()`") was originally stated without
qualification. That reads as implying a function like:

```fig
fn foo() -> int {
    return 5;
}
```

has a block whose value is `()` (trailing `;` after `return 5`), which is
wrong — `foo` returns `5`. The rule as stated didn't account for
diverging statements.

## Decision

The trailing-expression rule describes a block's value **when execution
reaches the closing `}` normally.** It doesn't apply to a statement that
unconditionally diverges — `return`, `break`, `continue` — even though
such a statement also ends in a `;`. `return`/`break`/`continue` have
their own type, `!` ("never"), meaning they never actually finish
evaluating — so control never reaches the point where "what is this
block's value" would even be asked. The `;` after `return 5` is ordinary
statement syntax, not a signal applied to the `5`.

## Rationale

This isn't a special case bolted onto the rule; it falls out of `!`
already being documented as a real type that unifies with anything. The
fix was to state the scope of the rule precisely and tie it back to `!`,
rather than to change any actual behavior — the underlying semantics
(what a `return`ing function actually returns) were never in question,
only the prose describing it.

See: `book/src/expressions/expressions-and-statements.md` ("The
trailing-expression rule", "`return`, `break`, and `continue` are
expressions too").
