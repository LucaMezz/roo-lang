# Identifiers and Keywords

## Identifiers

An identifier starts with a letter or underscore, followed by any number of
letters, digits, or underscores:

```text
identifier ::= (letter | "_") (letter | digit | "_")*
```

`_` alone is a special identifier — the **wildcard pattern** — used to
deliberately ignore a value:

```fig
let _ = compute_and_discard();

fn ignore_second(_: int, y: int) -> int {
    y
}
```

A name that starts with `_` but has more characters (`_unused`) is an
ordinary identifier that additionally suppresses "unused variable"
diagnostics.

### Naming conventions

fig follows Rust's naming conventions. They are conventions, not
grammar rules — the language doesn't reject code that violates them — but
idiomatic fig code follows them the way idiomatic Rust does:

| Item | Convention | Example |
|---|---|---|
| Variables, function/method names, module names | `snake_case` | `let item_count`, `fn parse_input` |
| Types (`struct`, `enum`, `trait`), generic type parameters | `UpperCamelCase` | `struct HttpRequest`, `trait Shape` |
| Bindings meant to be read as fixed/tunable values | `SCREAMING_SNAKE_CASE` | `let MAX_RETRIES: int = 5;` |
| Enum variants | `UpperCamelCase` | `enum Direction { North, South }` |

## Keywords

The following identifiers are reserved and can't be used as variable,
function, or type names.

### Strict keywords

Always reserved:

```text
as        break     continue  else      enum       false
fn        for       if        impl      in         let
loop      match     mod       pub       return     self
Self      struct    super     trait     true       type
use       where     while
```

### Reserved, currently unused

Reserved so they stay available if fig needs them later, but no current
fig syntax gives them meaning:

- `dyn` — see [Differences from Rust](../design/differences-from-rust.md).
- `const` — fig has no separate constant-binding form; an ordinary
  module-level `let` covers the need (see
  [Variables: Module-level bindings](../bindings/variables.md#module-level-bindings)).
  Reserved rather than freed up, in case fig later wants a binding form
  with a genuine compile-time-only-evaluation guarantee (for example, if
  const generics are ever added).

### Not keywords in fig

Words that are reserved keywords in Rust but have no meaning in fig, because
the feature they belong to doesn't exist, are **not** reserved and may be
used freely as identifiers: `unsafe`, `move`, `static`, `extern`, `ref`,
`box`, `async`, `await`, `yield`, `abstract`, `final`, `override`, `priv`,
`typeof`, `unsized`, `virtual`, `crate`, and `mut`. This is a deliberate
consequence of removing the features that needed them — see
[Differences from Rust](../design/differences-from-rust.md) for the full
list of removed concepts. `crate` specifically is freed up because fig has
modules but no crate-level compilation unit above them — see
[Modules and Visibility](../modules/modules.md). `mut` is freed up because
every binding is mutable by default and there's no immutable binding form
for it to distinguish from — see [Variables](../bindings/variables.md).

`Self` (capital-S) is reserved as the keyword referring to "the type this
`impl`/`trait` block is for" — see [Structs](../data-types/structs.md) and
[Traits](../abstraction/traits.md). `self` (lowercase) is reserved as the
name of a method's receiver parameter.
