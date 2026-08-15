# Type Inference

fig performs real, Hindley-Milner-style type inference — the same family
of inference OCaml and Haskell use, not just the local
"infer-this-one-`let`" kind most gradually-typed scripting languages mean
by the term. This is the single biggest thing that separates fig's type
system from a typical gradual type system, and it changes what leaving a
type annotation off actually *means*: omitting a type doesn't ask fig to
skip checking that position — it asks fig to work out the strongest
static type it can for it, checking everything it can prove, before ever
falling back to leaving something dynamically typed.

Concretely, that means an entire function — every parameter, and the
return type — can be left completely unannotated, and fig will still
give it a full, checked, statically-tracked type, including inferring
generic type parameters it was never told about. See
[Gradual Typing](gradual-typing.md) for what's left of "dynamically
typed" once inference is this strong (still real, just smaller than you
might expect), and [Design: Philosophy](../design/philosophy.md#gradual-typing-vs-strong-inference)
for why fig ended up here instead of at the more ordinary
gradual-typing design that inspired it.

## Where inference applies

A `let` binding's type is inferred from its initializer when the
initializer unambiguously determines one:

```fig
let count = 5;          // inferred: int
let ratio = 5.0;        // inferred: float
let label = "score";    // inferred: String
let items = [1, 2, 3];  // inferred: [int]
```

This is **not** the same as `count` being dynamically typed — once
inferred, `count`'s type is fixed to `int` for the rest of its scope,
exactly as if you had written `let count: int = 5;`:

```fig
let count = 5;
count = "oops"; // type error: expected `int`, found `String` — inferred,
                // not dynamic
```

## Untyped functions are inferred and generalized, not dynamically typed

This is the headline case. A function with no type annotations at all —
neither on its parameters nor on its return type — is still fully,
statically checked:

```fig
fn identity(x) {
    x
}
```

fig checks `identity`'s body (`x`), finds nothing that pins `x` down to
any particular concrete type, and — instead of giving up and treating
`x` as dynamically typed — concludes that `identity` works for *any*
type at all, and makes it **generic**. The type fig actually gives
`identity` is:

```fig
fn identity<T>(x: T) -> T
```

That's not an approximation or a summary — it is, internally, the exact
same type as if you had written the `<T>` yourself. fig's checker
represents both the same way: a function symbol with a list of generic
parameters and a signature built from them, instantiated fresh at every
call site. There's no way to tell, from inside the checker, that
`identity` was written without the `<T>` — the two are indistinguishable
after fig finishes reading `identity`'s definition. This is a deliberate
design goal, not a coincidence: inferred polymorphism and written-out
polymorphism are meant to be the same mechanism, so that leaving a
generic parameter off is never a *weaker* choice than writing it, only a
more concise one.

## Partial annotations only leave the gaps inferred

Annotating some parameters and not others is completely ordinary — fig
infers exactly the positions you didn't pin down, and checks the ones you
did:

```fig
fn describe(name: String, value) {
    print(name);
    print(value);
}
```

Here `name` is checked as `String`, exactly as written, while `value` is
inferred the same way `identity`'s `x` was — unconstrained by the body,
so it generalizes to its own type parameter:

```fig
fn describe<T>(name: String, value: T)
```

## Every call site instantiates independently

A generic function — inferred or explicit, no difference — gets a fresh
copy of its type variables at every call, so the same function can be
used at different types in the same program without them interfering
with each other:

```fig
fn identity(x) {
    x
}

let a = identity(5);      // this call: T = int
let b = identity("hi");   // this, separate call: T = String
```

Both calls are fine — `a: int` and `b: String` — because each call
instantiates `identity<T>(x: T) -> T` with its own fresh `T`, unrelated
to any other call's `T`. This is exactly how a generic function you wrote
`<T>` on by hand already behaves; inference doesn't change it.

## Explicit and inferred generics are the same feature

Since the two are represented identically, everything documented in
[Generics](../abstraction/generics.md) — trait bounds, `where` clauses,
multiple type parameters, defaults — is about the *shape* a generic
signature has, not about whether you wrote `<T>` or let fig infer it.
These two definitions produce the same kind of function, just arrived at
differently:

```fig
fn generic_identity<T>(x: T) -> T {
    x
}

fn identity(x) {
    x
}
```

The difference is only about *when* `T` gets introduced: `generic_identity`
declares it up front, so fig can check that the body actually works for
every possible `T` (writing `fn generic_identity<T>(x: T) -> T { 5 }`
would be a real type error — the body isn't allowed to assume `T` is
`int` just because that happens to typecheck). `identity` never declares
`T` — fig discovers, after checking the body, that nothing constrained
`x`'s type, and generalizes it after the fact. Once that's done, both
functions are `<T>(x: T) -> T`, checked and instantiated the same way at
every call site.

A trait bound only makes sense written explicitly, since it constrains
what the body is *allowed* to assume before checking it — inference can
generalize an unconstrained type, but it can't invent a bound you never
wrote:

```fig
fn largest<T: PartialOrd>(items: [T]) -> T {
    let best = items[0];
    for item in items {
        if item > best { // requires PartialOrd
            best = item;
        }
    }
    best
}
```

## Explicit generic type arguments (turbofish)

You can supply generic type arguments explicitly at a call site instead
of leaving fig to infer them — the same `::<...>` ("turbofish") syntax
Rust uses:

```fig
fn identity(x) {
    x
}

let a = identity::<int>(5); // fine — T pinned to int, and 5 is an int
```

If the turbofish's type disagrees with the argument actually passed,
that's a real type error, exactly as if you'd annotated a `let`:

```fig
let b = identity::<bool>(5); // type error: expected `bool`, found `int`
```

Once you write a turbofish at all, it's all-or-nothing: you must supply
exactly as many type arguments as the function has generic parameters —
no fewer, no more. Use `_` for any you want left to ordinary inference
rather than omitting them:

```fig
fn pair<A, B>(a: A, b: B) -> (A, B) {
    (a, b)
}

let p = pair::<int, bool>(1, true); // fine — both pinned explicitly
let q = pair::<int>(1, true);       // error: expected 2 generic
                                      // arguments, found 1
let r = pair::<_, bool>(1, true);   // fine — A left to inference (int),
                                      // B pinned explicitly
```

## Generic type aliases

A `type` alias can be generic too, and follows the exact same
generalize-once/instantiate-per-use model as a function:

```fig
type Pair<T, U> = (T, U);

let a: Pair<int, int> = (1, 2);          // explicit, checked
let b: Pair = (1, "two");                // bare — inferred as Pair<int, String>
let c: Pair<int, String> = (3, "three"); // explicit again, independently
```

Leaving the type arguments off (`Pair` on its own) works the same way an
untyped function parameter does: fig infers them from whatever context is
available — here, the tuple literal `Pair` is assigned. Using `Pair` with
the wrong number of explicit type arguments is a checked error, the same
message as a function's turbofish arity mismatch:

```fig
let d: Pair<int> = (1, 2); // error: expected 2 generic arguments, found 1
```

## Recursive and mutually recursive functions (planned)

Right now, a function that calls itself, or calls another function
declared in the same scope as it — even a plain, non-recursive helper
call — does **not** get generalized. It's still fully, correctly checked,
just as an ordinary, single, concrete type rather than a generic one:

```fig
fn identity_rec(x) {
    identity_rec(x) // self-referencing
}

let a = identity_rec(5);     // fine — first call fixes the type to int
let b = identity_rec("hi");  // type error: expected `int`, found `String`
```

Contrast that with a function that doesn't reference itself or a
sibling, like `identity` earlier in this chapter, which stays generic and
handles both calls fine. The restriction exists because generalizing a
function that's part of a recursive or mutually-recursive group correctly
requires analyzing the whole group together — checking every member,
then generalizing all of them at once over whatever's still free — rather
than one function at a time in declaration order. That analysis (a
call-graph strongly-connected-components pass) is planned but not yet
implemented, so fig conservatively treats any such function as
monomorphic instead of ever generalizing it incorrectly. Once it lands,
`identity_rec` above will infer to `identity_rec<T>(x: T) -> T`, and both
calls will typecheck — this is meant to become an ordinary part of the
inference story, not a permanent limitation.

## Ambiguous positions still generalize — they don't fall back to dynamic

Even a parameter your function's body never uses at all still gets a
real, checked generic type, rather than becoming dynamically typed for
lack of anything to pin it down:

```fig
fn ignore(x) {
    0
}
```

`x` is never read anywhere in `ignore`'s body, so nothing constrains its
type — but fig still generalizes it, the same way it would generalize any
other unconstrained parameter:

```fig
fn ignore<T>(x: T) -> int
```

`ignore(5)` and `ignore("hi")` are both fine, in the same program, for
the same reason `identity`'s two calls both were: each call instantiates
its own fresh `T`. This is the concrete difference from a gradual type
system that treats "nothing to infer from" as "make it dynamic" — fig
keeps looking for the strongest thing it can prove (here, "works for any
type") before it would ever give up on static checking altogether.

## Where inference does *not* apply yet

- **Struct and enum fields are not generalized.** A `struct`'s own type
  parameters (`struct Pair<T> { first: T, second: T }`) work exactly as
  described in [Generics](../abstraction/generics.md#generic-structs-and-enums)
  when written explicitly, but an unannotated field itself is not
  inferred from how the struct gets constructed elsewhere — it falls
  back to gradual typing, same as before. See
  [Gradual Typing: Structs](gradual-typing.md#structs-and-gradual-typing).
- **`let` bindings with no initializer** have nothing to infer from
  either, and need an explicit annotation to be statically typed — see
  [Gradual Typing](gradual-typing.md).
- **Self- and sibling-recursive functions**, as above — planned, not yet
  implemented.

## Overriding inference with `any`

Whether a position would be inferred to a concrete type or generalized
into a type parameter, `any` always overrides it and asks for genuinely
dynamic behavior instead:

```fig
fn identity(x: any) {
    x
}
```

Unlike a bare `x`, this `x` is never generalized — it's pinned to `any`,
exactly as written, the same way `let count: any = 5;` opts a `let`
binding out of inference. See
[Gradual Typing: The explicit `any` type](gradual-typing.md#the-explicit-any-type).
