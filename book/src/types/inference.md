# Type Inference

fig performs Rust-style local type inference for statically-typed code,
independently of gradual typing. These are two different mechanisms that
are easy to conflate:

- **Type inference** answers "what static type does this expression have,"
  when that type isn't written down but *is* fully determined by context.
- **Gradual typing** (previous chapter) answers "should this even be
  statically checked," when there's no annotation *and* no way to pin down a
  single static type from context alone.

## Where inference applies

A `let` binding's type is inferred from its initializer when the
initializer unambiguously determines one:

```fig
let count = 5;          // inferred: int
let ratio = 5.0;        // inferred: float
let label = "score";    // inferred: String
let items = [1, 2, 3];  // inferred: [int]
```

This is **not** the same as `count` being dynamically typed — once inferred,
`count`'s type is fixed to `int` for the rest of its scope, exactly as if
you had written `let count: int = 5;`:

```fig
let count = 5;
count = "oops"; // type error: expected `int`, found `String` — inferred,
                // not dynamic
```

Compare this to a genuinely untyped binding, which has no initializer-driven
type to infer — see [Gradual Typing](gradual-typing.md) — or to one that
opts out of inference explicitly with `any`, below.

Inference also fills in generic type arguments at call sites, exactly like
Rust:

```fig
fn identity<T>(x: T) -> T { x }

let n = identity(5); // T inferred as int; n: int
```

## Overriding inference with `any`

Inference is the default for a `let` with an initializer specifically
*because* fig has something local to infer from — not because static
typing is somehow mandatory there. If you want a binding to stay
dynamically typed despite having an inferable initializer, say so with
[`any`](gradual-typing.md#the-explicit-any-type):

```fig
let count: any = 5;
count = "now a string"; // fine — inference never ran; `any` opted out of it
```

Without the annotation, `count` would have inferred to `int` and this
reassignment would be a type error, per [above](#where-inference-applies).

## Where inference does *not* apply

- **Function parameters and return types are never inferred from usage.**
  Unlike a `let`'s initializer, there's no single local expression at a
  parameter's or return type's declaration site to infer from — a
  parameter's value comes from whichever call site happens to invoke it,
  and fig deliberately does not do whole-program, call-site-driven
  inference the way Hindley-Milner languages (OCaml, Haskell) do. An
  unannotated parameter or return type is therefore dynamically typed
  (gradual typing, not an inference failure) — matching Rust, which also
  requires every function signature to be fully written out.
- **Struct and enum field types are never inferred**, for the same reason:
  a field's value comes from wherever the type gets constructed, not from
  anything local to the field declaration. Every field is either annotated
  or dynamically typed, never guessed from how the type is constructed
  elsewhere.
- `let` bindings with **no initializer** have nothing to infer from either,
  and must carry an explicit annotation if you want them statically typed:

  ```fig
  let x: int; // ok — annotated, assigned later
  x = 5;

  let y;      // dynamically typed, not an inference failure — see
              // Gradual Typing
  ```

## Ambiguous cases

If an expression's static type genuinely can't be pinned down from local
context (for example, an empty array literal with no other usage in scope
to constrain its element type), and no annotation is given, the binding is
simply treated as dynamically typed rather than rejected — inference failure
falls back to gradual typing instead of being a hard compiler error. Adding
an annotation always resolves the ambiguity in favor of static checking:

```fig
let items: [int] = []; // annotation resolves the element type
```
