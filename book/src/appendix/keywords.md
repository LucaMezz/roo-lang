# Keyword Reference

## Strict keywords

Always reserved; can't be used as an identifier.

| Keyword | Purpose | See |
|---|---|---|
| `as` | Explicit type casts | [Casting and Conversion](../types/casting.md) |
| `break` | Exit a loop | [Loops](../control-flow/loops.md) |
| `continue` | Skip to the next loop iteration | [Loops](../control-flow/loops.md) |
| `else` | Alternate branch for `if`/`if let`/`let else` | [if and if let](../control-flow/if.md) |
| `enum` | Define an enum type | [Enums](../data-types/enums.md) |
| `false` | Boolean literal | [Literals](../lexical/literals.md) |
| `fn` | Define a function | [Functions](../functions/functions.md) |
| `for` | Iterate over a collection | [Loops](../control-flow/loops.md) |
| `if` | Conditional branch | [if and if let](../control-flow/if.md) |
| `impl` | Implement inherent methods or a trait | [Structs](../data-types/structs.md), [Traits](../abstraction/traits.md) |
| `in` | Part of `for pattern in expr` | [Loops](../control-flow/loops.md) |
| `let` | Declare a variable binding | [Variables](../bindings/variables.md) |
| `loop` | Unconditional loop | [Loops](../control-flow/loops.md) |
| `match` | Pattern-match an expression | [match](../control-flow/match.md) |
| `mod` | Declare a module | [Modules and Visibility](../modules/modules.md) |
| `pub` | Make an item visible outside its module | [Modules and Visibility](../modules/modules.md) |
| `return` | Return a value from a function early | [Functions](../functions/functions.md) |
| `self` | The receiver parameter of a method | [Structs](../data-types/structs.md) |
| `Self` | The implementing type, inside a `trait`/`impl` | [Traits](../abstraction/traits.md) |
| `struct` | Define a struct type | [Structs](../data-types/structs.md) |
| `super` | Path prefix referring to the parent module | [Modules and Visibility](../modules/modules.md) |
| `trait` | Define a trait | [Traits](../abstraction/traits.md) |
| `true` | Boolean literal | [Literals](../lexical/literals.md) |
| `type` | Associated type in a trait/impl | [Traits](../abstraction/traits.md) |
| `use` | Bring a path into scope | [Modules and Visibility](../modules/modules.md) |
| `where` | Trait-bound clause on a generic item | [Generics](../abstraction/generics.md) |
| `while` | Conditional loop | [Loops](../control-flow/loops.md) |

## Reserved, currently unused

Reserved so they stay available if roo needs them later, but no current
roo syntax gives them meaning.

| Keyword | Purpose |
|---|---|
| `dyn` | See [Differences from Rust](../design/differences-from-rust.md). |
| `const` | roo has no constant-binding form at all currently, module-scoped or otherwise — see [Variables: No module-level constants, for now](../bindings/variables.md#no-module-level-constants-for-now) — but the keyword stays reserved in case one is added later. |

## The wildcard identifier

`_` is a special identifier (the wildcard pattern), not a keyword — see
[Identifiers and Keywords](../lexical/identifiers-and-keywords.md).

## Rust keywords that are *not* reserved in roo

Because the corresponding features don't exist in roo, these Rust keywords
are ordinary, usable identifiers in roo — see
[Identifiers and Keywords: Not keywords in roo](../lexical/identifiers-and-keywords.md#not-keywords-in-roo):

```text
unsafe   move    static   extern   ref     box
async    await   yield    abstract final   override
priv     typeof  unsized  virtual  crate   mut
```

`mut` in particular isn't just unreserved — it never had meaning in roo to
begin with. Every binding is mutable by default and there's no way to
declare one immutable, so there's nothing left for a `mut` keyword to mark.
See [Variables](../bindings/variables.md).
