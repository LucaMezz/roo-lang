# Gradual Typing

Every type annotation in roo is optional. This chapter specifies what's
left of "dynamically typed" once you take into account how much roo
actually infers instead — which, as of [Type Inference](inference.md), is
most of what you'd expect to have to annotate, including entire function
signatures. Gradual typing and type inference are easy to conflate, so
it's worth being precise about the difference: inference is roo working
out a static type you didn't write down; gradual typing is what happens
in the positions where there's genuinely nothing to infer it from, or
where you explicitly asked for dynamic behavior with `any`.

## No annotation doesn't mean one fixed thing

In Rust, every binding has a static type, even when you don't write it out
— `let x = 5;` still gives `x` the fully static type `i32`, inferred by the
compiler. roo also infers types — in more places than you might expect,
covering entire function signatures via [real
generalization](inference.md#untyped-functions-are-inferred-and-generalized-not-dynamically-typed),
not just `let` — but not everywhere. Leaving a type annotation off
resolves one of two ways, depending on whether roo has something to infer
a type *from*:

- A `let` binding's initializer, or a function's own body, is a source roo
  can infer from — so roo infers a static type (possibly a generic one)
  and the position is statically checked from then on, exactly as if
  you'd written the annotation yourself. See [Type Inference](inference.md)
  for the full rules, including why an untyped function parameter is
  *not* the same case as a struct field below.
- A struct/enum field, or a `let` with no initializer at all, has no such
  source — nothing pins down a single type, and there's no function body
  to generalize over — so these stay **dynamically typed** when left
  unannotated, checked at run time instead, the way a variable in Lua or
  Python is.

```roo
let count = 5;    // inferred: int (a local source exists) — statically checked from here on
fn double(n) { n * 2 } // inferred: fn double<T>(n: T) -> T — see Type Inference
struct Item { label } // `label` has nothing to infer from — dynamically typed
```

This is a stronger inference story than TypeScript's (`let x = 5` infers
`number`) or Luau's (`local x = 5` infers `number`) — both only infer
`let`-shaped locals, and leave an untyped function parameter or return
type dynamically typed, which is what earlier versions of this design did
too. See [Design: Philosophy](../design/philosophy.md#gradual-typing-vs-strong-inference)
for why roo pushed further than that.

## The explicit `any` type

Sometimes you want a binding to stay dynamically typed even where roo
*would* infer a static type from it — for instance, a variable whose first
value happens to be a literal, but which you intend to hold different
kinds of values over its lifetime. Write `any` as the type to say so:

```roo
let count: any = 5;
count = "now a string"; // fine — `any` opted this binding out of inference
```

`any` is a real, nameable type in roo, with the same status Luau gives it
(`local x: any = 5` is real Luau) — it isn't a keyword, just a builtin type
name, exactly like `int` or `String`. Writing it anywhere a type is
expected means "dynamically typed here, unconditionally," overriding
whatever roo would otherwise have inferred. It's never *required* — a
position with nothing to infer from is already dynamic without it — but it
makes that intent explicit and readable, and it's the only way to opt a
`let` binding with an initializer *out* of inference.

## Adding an annotation turns checking on

The moment you write a type annotation, that binding, parameter, field, or
return value is statically checked, exactly as it would be in Rust:

```roo
let y: int = 5;
y = "now a string"; // type error: expected `int`, found `String`
```

```roo
fn double(n: int) -> int {
    n * 2
}

double("hi"); // type error: expected `int`, found `String`
```

Annotating one thing doesn't force you to annotate anything else. A
function can have some typed parameters and some untyped ones; a `struct`
can have some typed fields and some untyped ones — though these two
"untyped" cases aren't the same anymore. A field falls back to gradual
typing, this chapter's subject; a parameter gets inferred, [Type
Inference](inference.md)'s:

```roo
fn greet(name: String, title) {   // `title` is inferred, not dynamic —
    print(title);                 // roo generalizes it: <T>(name: String, title: T)
    print(name);
}

struct Config {
    label: String,
    note,   // `note` really is dynamically typed — no function body for
}          // roo to generalize a field from
```

## Return types are inferred too, not an exception

It's tempting to assume a function with no `-> Type` returns `()`, by
analogy with Rust, where that's exactly what happens. In roo, that
intuition turns out to be *closer* to right than it first looks, just for
a different reason: a return type left off isn't pinned to anything in
particular — it's inferred from the body, the same as an untyped
parameter is. If the body has no trailing expression, that inferred type
genuinely comes out to `()`, because that's what the block itself
evaluates to; if the body does have a trailing expression, the return
type infers to whatever that expression's type is:

```roo
fn log(message: String) {
    print(message);
} // inferred: fn log(message: String) -> ()  — body has no trailing value

fn describe(message: String) {
    message
} // inferred: fn describe(message: String) -> String — trailing value is message
```

Both are real, static, concrete return types — checked from here on
exactly as if you'd written `-> ()` or `-> String` yourself. See
[Functions: Return type](../functions/functions.md#return-type) for more,
and [Type Inference](inference.md) for what happens when the body's
trailing value doesn't pin down one single concrete type either (it
generalizes, the same as a parameter would).

## The boundary between typed and untyped code

Because typed and untyped code freely call into each other, roo needs a
rule for what happens at the seam. The rule mirrors Luau/TypeScript: a
dynamically-typed value is allowed to flow into a statically-typed slot
*without complaint at the boundary*, and is checked *when it's actually
used* in a way that would violate the annotation.

Because inference is now strong enough to give ordinary untyped function
parameters and return types real static types (see
[Type Inference](inference.md)), a genuinely dynamic value in roo today
almost always traces back to an explicit `any`, a struct/enum field, or a
`let` with no initializer and no annotation — the cases
[above](#no-annotation-doesnt-mean-one-fixed-thing) that gradual typing
still actually covers:

```roo
fn takes_int(n: int) -> int {
    n + 1
}

let dynamic_value: any = load_config_value(); // explicitly opted out of inference
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
same way to a function whose *return type* is genuinely dynamic (written
`-> any`, not just left off — see
[above](#return-types-are-inferred-too-not-an-exception)
for why an ordinary omitted return type doesn't qualify anymore), when
that function is used somewhere expecting a concrete `Fn(...) -> T`:

```roo
fn log(message: String) -> any { print(message); }

let handler: Fn(String) -> () = log; // allowed — no complaint at assignment
handler("hi");                        // fine: log's body has no trailing
                                        // expression, so it returns `()`
                                        // at runtime, matching `handler`'s
                                        // declared type
```

```roo
fn oops(message: String) -> any { message } // dynamic return, *does* return a value

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

Compare this to the *same-shaped* `log`/`oops` pair with the `-> any`
removed: without it, both return types are inferred from the body instead
(`()` and `String` respectively, per
[above](#return-types-are-inferred-too-not-an-exception)),
and `let handler: Fn(String) -> () = oops;` becomes a static type error
right at that `let` — no runtime check needed, because there's no longer
anything dynamic in the picture for a runtime check to defer to.

## Structs and gradual typing

An untyped field behaves like an untyped variable: it can hold anything, and
is checked dynamically at each use. Writing `any` instead of leaving the
field bare means exactly the same thing, more explicitly:

```roo
struct Config {
    max_retries: int,   // statically checked
    payload,             // dynamically typed — any value at all
    label: any,           // same as `payload`, just spelled out
}
```

## What gradual typing is *not*

- It is **not** optional *safety*. A statically-annotated `int` parameter is
  just as strongly checked in roo as in Rust — gradual typing only affects
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
  though both can be triggered by leaving an annotation off, and inference
  covers far more ground than it might first appear to (a whole function
  signature, generic parameters included — not just a `let`). Gradual
  typing is what's left over: no annotation *and* nothing to infer from
  (a struct/enum field, a `let` with no initializer), or `any` explicitly
  asking for dynamic behavior regardless of what could otherwise be
  inferred.

## Why this design

roo is meant to work well both as a quick, throwaway script (few or no
annotations) and as a large, maintained codebase (fully annotated,
behaves like Rust), with a smooth path between the two — the same
argument Luau and TypeScript make for gradual typing over their fully
dynamic ancestors (Lua and JavaScript). Where roo departs from that
argument is in *how* the "few or no annotations" end stays approachable:
Luau and TypeScript get there by falling back to dynamic typing wherever
nothing's annotated; roo gets there by inferring as much as it possibly
can first, only actually landing on "dynamically typed" in the narrower
set of places [Type Inference](inference.md) can't reach yet (struct/enum
fields today) or where `any` asks for it on purpose. See
[Design: Philosophy](../design/philosophy.md#gradual-typing-vs-strong-inference)
for the reasoning behind that choice.

roo doesn't need a separate "strict mode" pragma either way: because
annotations are opt-in per binding rather than per file, a single file
can mix fully typed, fully inferred, and fully untyped code freely, and
the checked/unchecked boundary is exactly "is this position pinned to a
concrete or generic type — by inference or by annotation — or is it
dynamic, whether by having nothing to infer from or by an explicit
`any`."
