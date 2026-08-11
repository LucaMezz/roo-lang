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
| `mut` | Mark a binding as mutable | [Variables](../bindings/variables.md) |
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

Reserved so they stay available if fig needs them later, but no current
fig syntax gives them meaning.

| Keyword | Purpose |
|---|---|
| `dyn` | See [Differences from Rust](../design/differences-from-rust.md). |
| `const` | fig has no separate constant-binding form — see [Variables: Module-level bindings](../bindings/variables.md#module-level-bindings) — but the keyword stays reserved in case a form with a genuine compile-time-only-evaluation guarantee is added later. |

## The wildcard identifier

`_` is a special identifier (the wildcard pattern), not a keyword — see
[Identifiers and Keywords](../lexical/identifiers-and-keywords.md).

## Rust keywords that are *not* reserved in fig

Because the corresponding features don't exist in fig, these Rust keywords
are ordinary, usable identifiers in fig — see
[Identifiers and Keywords: Not keywords in fig](../lexical/identifiers-and-keywords.md#not-keywords-in-fig):

```text
unsafe   move    static   extern   ref     box
async    await   yield    abstract final   override
priv     typeof  unsized  virtual  crate
```
