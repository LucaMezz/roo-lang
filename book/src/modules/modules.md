# Modules and Visibility

fig organizes code into a tree of **modules**, the same nesting/privacy
model Rust uses.

## Declaring a module

An inline module groups items under a named namespace:

```fig
mod shapes {
    struct Circle { radius: float }

    fn describe(c: Circle) -> String {
        "a circle"
    }
}
```

Items are accessed through the module path with `::`:

```fig
let c = shapes::Circle { radius: 1.0 };
```

A module can also be declared to live in another file, `mod name;`,
resolved to a file (or directory) named `name` alongside the current one —
mirroring how Rust splits `mod foo;` across files, with the exact file/
directory resolution rules left to the implementation.

Unlike Rust, fig has no `crate` — no notion of a compilation unit sitting
above a module tree, and so no `crate::` path root and no `pub(crate)`
visibility either (see [Differences from Rust](../design/differences-from-rust.md)).
A fig program is just a module tree, resolved once, in full, by the
transpiler. This isn't a simplification made for its own sake: since fig
currently compiles by transpiling to Luau (see
[Ambient Modules](ambient-modules.md)), every `mod`/`use` among a program's
own fig files is resolved statically at compile time, and the whole tree is
flattened into the emitted Luau — there's no runtime module loader
involved for fig-authored code, so there was never a second, coarser unit
above "module" for `crate` to usefully name.

## Visibility

Every item — `fn`, `struct`, `enum`, `trait`, `mod` — is **private
by default**: visible within the module it's declared in, and any
descendant module, but not to anything outside it. A module-level `let`
binding (see [Variables: Module-level bindings](../bindings/variables.md#module-level-bindings))
follows the same rule and can be marked `pub` too, which is how fig exports
a fixed value from a module — Rust would use `pub const` for this; fig just
uses `pub let`.

```fig
mod shapes {
    struct Circle { radius: float } // private — not visible outside `shapes`

    pub let DEFAULT_RADIUS: float = 1.0; // pub — visible from outside

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

```fig
pub struct Circle {
    pub radius: float, // visible outside `shapes`
    cached_area: float, // private, even though Circle itself is pub
}
```

## `use`

`use` brings an item into scope under a shorter name, so it doesn't need to
be written out with its full path every time:

```fig
use shapes::Circle;

let c = Circle { radius: 1.0 }; // instead of shapes::Circle { ... }
```

`use` supports the same grouping and renaming forms as Rust:

```fig
use shapes::{Circle, area};
use shapes::Circle as Round;
use shapes::*; // glob import — brings every public item into scope
```

## Paths

A path is a `::`-separated sequence of module/item names. An unqualified
path (`shapes::Circle`) resolves from the root of the current module tree
by default — the same rule modern Rust uses for a bare path that isn't a
local item or `use`. `self::` and `super::` are available to be explicit
about relative resolution:

```fig
shapes::Circle // resolved from the root of the module tree
self::Circle    // explicitly relative to the current module
super::Circle    // relative to the parent module
```

## Ambient modules

Everything on this page describes modules whose code is written in fig.
fig also has a second, closely related kind of module — one whose items
have no fig implementation at all, because they're provided by whatever is
hosting the script (for fig-engine, code written in Rust). See
[Ambient Modules](ambient-modules.md).
