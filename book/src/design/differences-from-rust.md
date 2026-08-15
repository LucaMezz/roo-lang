# Differences from Rust

This page is the canonical list of every place fig's design departs from
Rust's, in one place, for readers who already know Rust and want the diff.
Everything here is explained in more depth in its own chapter; this is the
index.

## Removed entirely

These concepts have no equivalent in fig, because they exist in Rust
specifically to manage memory and hardware without a garbage collector or
runtime, a job fig's runtime does for you.

| Rust concept | Status in fig |
|---|---|
| Ownership & the borrow checker | Removed. See [The Value Model](values-and-mutation.md). |
| `&T`, `&mut T` (references/borrows) | Removed. Function parameters, `self`, and bindings just name a type; sharing vs. copying is determined by [value vs. reference types](values-and-mutation.md), not by an explicit `&`. |
| Lifetimes (`'a`, `'static`, lifetime elision) | Removed entirely, including from loop labels — see [Loops](../control-flow/loops.md) for fig's non-lifetime-shaped label syntax. |
| `unsafe` blocks/functions/traits | Removed. There is no operation in fig that is memory-unsafe. |
| Raw pointers (`*const T`, `*mut T`) | Removed. |
| Manual memory management, custom/explicit allocators (`Box::new` as allocation, `#[global_allocator]`) | Removed. All reference-typed values are managed by the fig runtime automatically. |
| `extern`, `#[no_mangle]`, `#[repr(C)]`, and other FFI machinery | Removed. fig has no C ABI story at the language level. Instead, a host embedding fig exposes native functionality as an [ambient module](../modules/ambient-modules.md) — an ordinary fig module whose function bodies are simply omitted. |
| `crate` keyword, `crate::` path root, `pub(crate)` | Removed. fig has modules, but nothing sits above the module tree the way a crate sits above modules in Rust — a fig program *is* its module tree, resolved once, in full, at compile time. See [Modules and Visibility](../modules/modules.md). |
| `macro_rules!`, procedural/derive macros | Removed. There is no macro system, and no built-in attribute has compiler-recognized meaning the way `#[derive(...)]` does in Rust. See [Structs](../data-types/structs.md) and [Enums](../data-types/enums.md) for how the most common derive use cases (equality, display) are handled instead. |
| `Rc<T>`, `Arc<T>`, `RefCell<T>`, `Cell<T>`, `Mutex<T>`, `Box<T>` as a distinct owning pointer | Removed as language concepts. Every reference type in fig already behaves like a shared, mutable, runtime-managed pointer, so none of these wrapper types are needed to get that behavior. |
| `static` items, interior mutability of globals | Removed. Use an ordinary [module-level `let`](../bindings/variables.md#module-level-bindings) instead. |
| Threads, `Send`/`Sync`, atomics | Not part of the language. Concurrency (if any) is a future runtime/standard-library concern, not language syntax. |
| `mut`, immutable-by-default bindings | Removed. Every `let`, function/closure parameter, and `self` is reassignable and writable-through by default, with no way to declare one immutable. See [Variables](../bindings/variables.md) and [The Value Model](values-and-mutation.md#every-binding-is-mutable). |

## Simplified

These concepts exist in fig, but in a smaller form than Rust's, because the
full Rust version's complexity mostly serves memory-layout or systems-level
precision that a scripting language doesn't need.

| Rust | fig | Why |
|---|---|---|
| `i8`/`i16`/`i32`/`i64`/`i128`/`isize` and `u8`/.../`usize`, `f32`/`f64` | One `int` and one `float` type. | Choosing an exact bit width is a memory-layout/performance concern, the same category as manual memory management. See [Primitive Types](../types/primitives.md). |
| `[T; N]` (fixed-size array) vs. `&[T]` (slice) vs. `Vec<T>` (growable, owned) | One `[T]` array type: always growable, always a reference type, indexed the same way `Vec<T>` is. | The three-way split exists so Rust can choose stack vs. heap layout and enforce borrowing on views into a buffer. Without ownership/borrowing there's no distinction left to preserve. See [Arrays and Tuples](../types/arrays-and-tuples.md). |
| `str` (borrowed) vs. `String` (owned) | One `String` type. | Same reasoning as arrays — the split exists to distinguish a borrowed view from an owned buffer. See [Strings and Characters](../types/strings-and-chars.md). |
| `self` / `&self` / `&mut self` | Just `self`. | Since parameters are already references for reference types, and every binding is mutable, there's no separate "borrow `self` mutably" mode to opt into — every method can write through `self` already. See [Structs](../data-types/structs.md#methods). |
| `Fn` / `FnMut` / `FnOnce`, `move` closures | One closure kind, capturing the same way any function parameter binds (see [The Value Model](values-and-mutation.md)). | The three-trait split exists to track *how* a closure interacts with ownership of its captures; with no ownership, there's one behavior. See [Closures](../functions/closures.md). |
| `dyn Trait`, trait objects, object safety, `Box<dyn Trait>` | A trait name used directly in type position means "anything implementing this trait," dynamically checked, no `dyn` or box needed. | Trait objects in Rust exist to put an unsized, dynamically-dispatched value behind a pointer explicitly. Since reference types are already runtime-managed pointers, that's the default behavior for trait-typed values in fig — no separate spelling required. See [Traits](../abstraction/traits.md). |
| Enum explicit discriminants (`enum E { A = 1 }`) | Not supported. | This exists in Rust primarily for FFI/C interop layout control. |
| `const` (separate keyword, compile-time-only-evaluable initializer, required type annotation) | An ordinary module-level `let` — no separate keyword. | `const`'s compile-time-evaluation guarantee backs fixed-size array lengths, `static` initialization order, and const generics in Rust — none of which fig has. Without a use for the guarantee, a second binding form isn't earning its keep; `let` already covers "a fixed, tunable, module-level value," by convention rather than compiler-enforced immutability. (Luau itself has no separate `const` form either — just `local x <const> = value`.) `const` stays a reserved word in case fig later wants that guarantee back. See [Variables: Module-level bindings](../bindings/variables.md#module-level-bindings). |
| `#[attribute]`s | Same `#[...]` syntax, but inert at the language level — fig itself assigns no meaning to any annotation name, built-in or otherwise. All interpretation comes from the host embedding fig, which registers and reads them back. No macro expansion, no compiler built-ins like `cfg`/`repr`. See [Annotations](../annotations/annotations.md). |

## Kept as-is

Everything else you'd expect from Rust is kept, deliberately unchanged in
spelling and meaning, including: `let`/shadowing,
`if`/`else` and `if let`/`let else` as expressions, `loop`/`while`/`while
let`/`for`, `match` with full pattern matching (destructuring, ranges,
guards, `|`-patterns, `@`-bindings), `struct`s (named/tuple/unit), `enum`s
with data-carrying variants, `impl` blocks, `trait`s (including default
methods and associated types), generics with trait bounds and `where`
clauses, operator overloading via trait implementation, modules (`mod`,
`use`, `pub`, `pub(super)`, `self::`/`super::` paths — everything except
`crate`, see above), the full standard operator set, `as` casts, and
`Result`/`Option`/the `?` operator as the error-handling model (concrete
types TBD by the standard library, contract described in
[Error Handling](../errors/error-handling.md)).

## Added beyond Rust

Two things fig has that Rust does not:

- **Gradual typing.** Every type annotation is optional. See
  [Gradual Typing](../types/gradual-typing.md).
- **Ambient modules.** A module made of bodyless function signatures,
  describing an API a host embedding fig provides to scripts, used with
  ordinary module syntax and no FFI ceremony. Rust has nothing quite like
  this — its nearest relative, `extern` blocks, is one of the FFI features
  fig removes. See [Ambient Modules](../modules/ambient-modules.md).
