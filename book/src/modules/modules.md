# Modules and Visibility

roo organizes code into a tree of **modules**, the same nesting/privacy
model Rust uses.

## Declaring a module

An inline module groups items under a named namespace:

```roo
mod shapes {
    struct Circle { radius: float }

    fn describe(c: Circle) -> String {
        "a circle"
    }
}
```

Items are accessed through the module path with `::`:

```roo
let c = shapes::Circle { radius: 1.0 };
```

A module can also be declared to live in another file, `mod name;`,
resolved to a file (or directory) named `name` alongside the current one —
mirroring how Rust splits `mod foo;` across files, with the exact file/
directory resolution rules left to the implementation.

Unlike Rust, roo has no `crate` — no notion of a compilation unit sitting
above a module tree, and so no `crate::` path root and no `pub(crate)`
visibility either (see [Differences from Rust](../design/differences-from-rust.md)).
A roo program is just a module tree, resolved once, in full, by the
transpiler. This isn't a simplification made for its own sake: since roo
currently compiles by transpiling to Luau (see
[Ambient Modules](ambient-modules.md)), every `mod`/`use` among a program's
own roo files is resolved statically at compile time, and the whole tree is
flattened into the emitted Luau — there's no runtime module loader
involved for roo-authored code, so there was never a second, coarser unit
above "module" for `crate` to usefully name.

## Visibility

Every item — `fn`, `struct`, `enum`, `trait`, `mod` — is **private
by default**: visible within the module it's declared in, and any
descendant module, but not to anything outside it. `let` isn't on that
list — a module only holds items, never bindings (see
[Variables: No module-level bindings](../bindings/variables.md#no-module-level-bindings))
— so there's no `pub let` counterpart to Rust's `pub const` either. A
module exposes a fixed value the same way it exposes any other computed
one: as a `pub fn` that returns it.

```roo
mod shapes {
    struct Circle { radius: float } // private — not visible outside `shapes`

    pub fn default_radius() -> float { 1.0 } // pub — visible from outside

    pub fn area(c: Circle) -> float { // pub — visible from outside
        3.14159 * c.radius * c.radius
    }
}
```

`pub` makes an item visible outside its defining module (to anything that
can reach the module itself). Two narrower forms are also available:

| Visibility | Meaning |
|---|---|
| *(none)* | Visible in the defining module and its descendants only |
| `pub` | Visible anywhere the module itself is reachable from |
| `pub(super)` | Visible to the parent module (and its descendants) |

Struct fields follow the same rule independently of the struct itself: a
`pub` struct can still have private fields, visible only for construction
and field access from within the defining module.

```roo
pub struct Circle {
    pub radius: float, // visible outside `shapes`
    cached_area: float, // private, even though Circle itself is pub
}
```

## `use`

`use` brings an item into scope under a shorter name, so it doesn't need to
be written out with its full path every time:

```roo
use shapes::Circle;

let c = Circle { radius: 1.0 }; // instead of shapes::Circle { ... }
```

`use` supports the same grouping and renaming forms as Rust:

```roo
use shapes::{Circle, area};
use shapes::Circle as Round;
use shapes::*; // glob import — brings every public item into scope
```

By default, a `use` only brings a name into scope for the module it's
written in — it doesn't make that name reachable through *that* module's
own path. `pub use` does: it re-exports the imported item, so anything
that can reach the module doing the `pub use` can reach the item through
it too, as if it had been declared there directly:

```roo
mod shapes {
    pub mod circle {
        pub struct Circle { pub radius: float }
    }

    pub use circle::Circle; // re-exported as `shapes::Circle`
}

let c = shapes::Circle { radius: 1.0 }; // works — no need to know
                                          // about `shapes::circle` at all
```

This is how a module presents a flatter, more convenient public path than
its actual internal structure — callers of `shapes::Circle` don't need to
know `circle` is a separate inner module at all. `pub use` supports the
same grouping, renaming, and glob forms as plain `use`.

## Paths

A path is a `::`-separated sequence of module/item names. An unqualified
path (`shapes::Circle`) resolves from the root of the current module tree
by default — the same rule modern Rust uses for a bare path that isn't a
local item or `use`. `self::` and `super::` are available to be explicit
about relative resolution:

```roo
shapes::Circle // resolved from the root of the module tree
self::Circle    // explicitly relative to the current module
super::Circle    // relative to the parent module
```

## Ambient modules

Everything on this page describes modules whose code is written in roo.
roo also has a second, closely related kind of module — one whose items
have no roo implementation at all, because they're provided by whatever is
hosting the script (for roo-engine, code written in Rust). See
[Ambient Modules](ambient-modules.md).
