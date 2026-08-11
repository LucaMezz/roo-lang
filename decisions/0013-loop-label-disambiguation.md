# 0013. `break`/`continue`'s identifier is always a label if one's in scope

**Status:** Accepted

## Context

fig deliberately spells loop labels as plain identifiers (`outer: for ...
{ break outer; }`) rather than Rust's `'outer`, specifically so the
syntax doesn't visually imply lifetimes exist (fig has none). That choice
reopens an ambiguity Rust's `'` sigil exists partly to avoid: `break x;`
could mean "break the loop labeled `x`" or "break the innermost loop with
the value of variable `x`," and nothing in the syntax distinguishes them
if both a label and a variable named `x` are in scope at the same point.

Two directions were considered: a deterministic precedence rule with no
new syntax, or inventing dedicated escape-hatch syntax (e.g., requiring
parentheses to force the value interpretation).

## Decision

No new syntax. A deterministic rule: **the identifier right after
`break`/`continue` is always the label, if one with that name is in
scope — never a value expression.** This shadows any same-named variable
in that one position, the same flavor of precedence rule as ordinary name
shadowing elsewhere in the language.

## Rationale

The collision needed to actually trigger this — a variable and an
enclosing loop label sharing the exact same name, at a point where you
also want to `break` with that variable's value — is rare, and a loop
label and a nearby variable sharing a name is already confusing style
independent of this rule. Not judged worth inventing dedicated syntax
for; renaming either the variable or the label always resolves it.

See: `book/src/control-flow/loops.md` ("Labels vs. values named the same
thing"), `book/src/appendix/grammar.md`.
