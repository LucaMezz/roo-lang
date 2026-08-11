# Gradual Typing

This is fig's one major addition on top of "a subset of Rust": every type
annotation in the language is optional. This chapter specifies exactly what
that means.

## No annotation doesn't mean one fixed thing

In Rust, every binding has a static type, even when you don't write it out
— `let x = 5;` still gives `x` the fully static type `i32`, inferred by the
compiler. fig also infers types in some places, but not all — leaving a
type annotation off resolves one of two ways, depending on whether fig has
something local and unambiguous to infer a type *from*:

- A `let` binding's initializer is a local, unambiguous source — so fig
  infers a static type from it, and the binding is statically checked from
  then on, exactly as if you'd written the annotation yourself. See
  [Type Inference](inference.md) for the full rules.
- A function parameter, a struct/enum field, or a function's return type
  has no such source at its declaration site — nothing there pins down a
  single type — so these are **dynamically typed** when left unannotated,
  checked at run time instead, the way a variable in Lua or Python is.

```fig
let count = 5;          // inferred: int (a local source exists) — statically checked from here on
fn double(n) { n * 2 } // `n` has nothing local to infer from — dynamically typed
```

This is the same shape TypeScript's inference (`let x = 5` infers
`number`) and Luau's inference (`local x = 5` infers `number`) already
have — neither treats "no annotation" as a single, uniform "always
dynamic" rule either.

## The explicit `any` type

Sometimes you want a binding to stay dynamically typed even where fig
*would* infer a static type from it — for instance, a variable whose first
value happens to be a literal, but which you intend to hold different
kinds of values over its lifetime. Write `any` as the type to say so:

```fig
let count: any = 5;
count = "now a string"; // fine — `any` opted this binding out of inference
```

`any` is a real, nameable type in fig, with the same status Luau gives it
(`local x: any = 5` is real Luau) — it isn't a keyword, just a builtin type
name, exactly like `int` or `String`. Writing it anywhere a type is
expected means "dynamically typed here, unconditionally," overriding
whatever fig would otherwise have inferred. It's never *required* — a
position with nothing to infer from is already dynamic without it — but it
makes that intent explicit and readable, and it's the only way to opt a
`let` binding with an initializer *out* of inference.

## Adding an annotation turns checking on

The moment you write a type annotation, that binding, parameter, field, or
return value is statically checked, exactly as it would be in Rust:

```fig
let y: int = 5;
y = "now a string"; // type error: expected `int`, found `String`
```

```fig
fn double(n: int) -> int {
    n * 2
}

double("hi"); // type error: expected `int`, found `String`
```

Annotating one thing doesn't force you to annotate anything else. A
function can have some typed parameters and some untyped ones; a `struct`
can have some typed fields and some untyped ones:

```fig
fn greet(name: String, title) {   // `title` is dynamically typed
    print(title);
    print(name);
}
```

## Return types are not an exception

It's tempting to assume a function with no `-> Type` returns `()`, by
analogy with Rust, where that's exactly what happens. In fig it isn't: a
return type has no local source to infer from (see
[above](#no-annotation-doesnt-mean-one-fixed-thing)), so an omitted one is
dynamically typed, not statically pinned to `()` — writing `-> any` makes
that explicit, if you want it spelled out. If you want fig to guarantee a
function returns nothing, write `-> ()` instead. See
[Functions: Return type](../functions/functions.md#return-type) for the
full explanation, including why this doesn't change what a function
actually *returns* at runtime — only whether that return value is
statically checked.

## The boundary between typed and untyped code

Because typed and untyped code freely call into each other, fig needs a
rule for what happens at the seam. The rule mirrors Luau/TypeScript: a
dynamically-typed value is allowed to flow into a statically-typed slot
*without complaint at the boundary*, and is checked *when it's actually
used* in a way that would violate the annotation.

```fig
fn takes_int(n: int) -> int {
    n + 1
}

let dynamic_value = load_config_value(); // untyped — could be anything
takes_int(dynamic_value); // allowed statically; checked at the call
                           // — a runtime type error is raised here if
                           // `dynamic_value` doesn't actually hold an int
```

Going the other direction — a statically-typed value used where no
annotation exists — is always fine, since a typed value is automatically a
valid value of the dynamic world too; every type is a subtype of `any`.

### The same boundary applies to a function's return type

A function or closure is an ordinary value (see
[Functions: Functions as values](../functions/functions.md#functions-as-values)),
so the boundary rule above isn't just about parameters — it applies the
same way to a function whose *return type* is dynamic, when that function
is used somewhere expecting a concrete `Fn(...) -> T`:

```fig
fn log(message: String) { print(message); } // dynamic return

let handler: Fn(String) -> () = log; // allowed — no complaint at assignment
handler("hi");                        // fine: log's body has no trailing
                                        // expression, so it returns `()`
                                        // at runtime, matching `handler`'s
                                        // declared type
```

```fig
fn oops(message: String) { message } // dynamic return, *does* return a value

let handler: Fn(String) -> () = oops; // still allowed at assignment
handler("hi"); // runtime type error here: `oops` actually returned a
                // `String`, but `handler`'s declared type says `()`
```

The check happens **at the call**, not wherever the caller later uses the
result — the same checkpoint the parameter case above already uses. This
matters because `()` (and dynamic values generally) are easy to discard
without ever really "using" them; if the check were deferred to that
point, a call whose result goes unused would silently skip it, and
`Fn(...) -> T` would stop being a promise the type system actually keeps.
Checking at the call keeps it honest.

## Structs and gradual typing

An untyped field behaves like an untyped variable: it can hold anything, and
is checked dynamically at each use. Writing `any` instead of leaving the
field bare means exactly the same thing, more explicitly:

```fig
struct Config {
    max_retries: int,   // statically checked
    payload,             // dynamically typed — any value at all
    label: any,           // same as `payload`, just spelled out
}
```

## What gradual typing is *not*

- It is **not** optional *safety*. A statically-annotated `int` parameter is
  just as strongly checked in fig as in Rust — gradual typing only affects
  code with no annotation at all, and the boundary-crossing behavior above.
- It is **not** duck typing. Struct and enum types are still nominal —
  a `Point` and a same-shaped `Vector2` are different types even when typed
  code is involved. Gradual typing only concerns *whether* a check happens,
  not *what* it means for two static types to be compatible.
- It does **not** change generics, traits, or any other part of the static
  type system — those all work exactly as documented in
  [Generics](../abstraction/generics.md) and [Traits](../abstraction/traits.md)
  whenever they're used with type annotations present.
- It is **not** the same mechanism as [type inference](inference.md), even
  though both can be triggered by leaving an annotation off. Inference
  produces a static type from a local source (today, only a `let`
  initializer); gradual typing is what happens when there's no annotation
  *and* no local source, or when `any` explicitly asks for it regardless.

## Why this design

fig is meant to work well both as a quick, throwaway script (no annotations,
behaves like Lua) and as a large, maintained codebase (fully annotated,
behaves like Rust), with a smooth path between the two — exactly the
argument Luau and TypeScript make for gradual typing over their fully
dynamic ancestors (Lua and JavaScript). Unlike those two languages, though,
fig doesn't need a separate "strict mode" pragma: because annotations are
opt-in per binding rather than per file, a single file can mix fully typed
and fully untyped code freely, and the checked/unchecked boundary is exactly
"is this position pinned to a concrete type — by inference or by
annotation — or is it dynamic, whether by having nothing to infer from or
by an explicit `any`."
