# Annotations

fig has a small, simplified version of Rust's attribute system: `#[...]`,
written directly above the thing it describes.

```fig
#[replicated]
struct Position {
    x: float,
    y: float,
}
```

The word "simplified" is doing real work here. Rust's attributes drive
`macro_rules!`, procedural macros, and compiler built-ins like
`#[derive(...)]` and `#[repr(C)]` — fig has none of that (see
[Differences from Rust](../design/differences-from-rust.md)). An
annotation in fig is inert as far as the fig compiler is concerned: it's
structured data attached to a declaration, nothing more. fig itself never
inspects, expands, or acts on an annotation's contents. All meaning comes
from *outside* the script — from the host embedding fig — which is the
subject of this chapter.

## Syntax

An annotation is a path, optionally followed by more detail, in one of
three shapes:

```fig
#[replicated]                    // a bare word
#[range = 100]                   // a name paired with a value
#[range(min = 0, max = 100)]     // a name paired with a list of arguments
```

These aren't three unrelated forms — they're the same underlying
`path (= literal | ( ... ))?` shape, just with the trailing part omitted,
a single value, or a list.

## Nesting

The arguments inside the list form aren't restricted to bare
words or literals — each one is a full annotation body in its own right,
which means they nest to whatever depth is useful:

```fig
#[serialize(rename = "pos", skip_if(default, empty))]
struct Position {
    x: float,
    y: float,
}
```

Here `serialize`'s argument list contains `rename = "pos"` (a name-value
pair) and `skip_if(default, empty)` (a nested list, whose own arguments
are two bare words). There's no fixed limit on how deep this goes — a
list argument can itself contain a list argument, and so on.

## Where they attach

Annotations sit directly above the declaration they describe — most
usefully structs, enums, individual enum variants, and struct fields,
which is where the host is most likely to want to attach metadata to a
script's data model:

```fig
#[component]
struct Health {
    #[range(min = 0, max = 100)]
    current: int,

    #[range(min = 0, max = 100)]
    max: int,
}

#[replicated]
enum DamageKind {
    #[audio_cue = "hit_physical"]
    Physical,
    #[audio_cue = "hit_fire"]
    Fire,
}
```

An annotation always describes the single declaration immediately
following it — never a whole block or a group of declarations at once.

## The host defines what they mean

fig deliberately doesn't ship a fixed vocabulary of built-in annotations
the way Rust ships `derive`, `cfg`, and `repr`. Instead, the host
embedding fig — for [fig-engine](https://github.com/LucaMezz/fig-engine),
the motivating case this feature was designed around — registers the
annotation names it understands and reads them back off a script's items
when it needs to. `#[replicated]` and `#[range(min = 0, max = 100)]` above
aren't language features; they're names an engine happens to have chosen,
in exactly the same spirit as ambient modules (see
[Ambient Modules](../modules/ambient-modules.md)) letting a host describe
its own API surface rather than fig hard-coding one.

Concretely, this makes annotations useful for exactly the kind of
metadata a game engine wants without touching the language: marking a
component field as networked, giving the editor a slider range for a
number, naming the serialized form of a struct, tagging an enum variant
with an asset to play. None of that is fig's concern — the language's job
ends at parsing `#[...]` into structured data and attaching it to the
right declaration; interpreting it is entirely up to whatever's reading
it back.

fig doesn't mandate *when* the host reads annotations, either — that's a
property of the embedding, not the language. A reasonable implementation
might inspect them once, at load time (fig-engine, for instance, would do
this after transpiling a script but before running it, while it still has
access to the full declaration — the same point ambient module bindings
get wired up), rather than every time a script value is touched at
runtime.

## Outer vs. inner form

Every example so far has been the *outer* form, `#[...]`, which describes
the declaration immediately after it. fig's grammar also has room for an
*inner* form, `#![...]`, which (as in Rust) would describe the thing the
annotation is written *inside of* rather than the thing that follows —
typically used at the top of a module to annotate the module itself,
since there's no "next declaration" to attach to at that position. Which
positions accept `#![...]` in fig, and what it means there, isn't settled
yet — treat the inner form as reserved, not yet meaningful, until that's
decided.
