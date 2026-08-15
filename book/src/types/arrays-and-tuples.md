# Arrays and Tuples

## Arrays

An array is an ordered, growable collection of values of a single type,
written `[T]` as a type and `[e1, e2, ...]` as a literal. Like `String`,
an array is a **reference type** — see
[The Value Model](../design/values-and-mutation.md).

```roo
let numbers: [int] = [1, 2, 3];
let names = ["Ada", "Grace"]; // [String], inferred

names[0];          // indexing: "Ada"
names[0] = "Judith"; // assignment through an index
```

Rust splits this into three types — `[T; N]` (a fixed-size, stack-allocated
array), `&[T]` (a borrowed slice/view), and `Vec<T>` (a growable, owned,
heap-allocated buffer) — because those distinctions matter to a borrow
checker managing stack vs. heap layout. roo has none of that machinery, so
all three collapse into a single, always-growable, always-reference-typed
`[T]`, functionally closest to `Vec<T>`. See
[Differences from Rust](../design/differences-from-rust.md).

### Indexing

Indexing uses `[]` with an `int` index, starting at `0`. Indexing
out-of-bounds is a runtime error, exactly as in Rust.

```roo
let first = numbers[0];
let out_of_bounds = numbers[99]; // runtime error
```

### Iteration

Arrays can be iterated with a `for` loop — see [Loops](../control-flow/loops.md):

```roo
for n in numbers {
    print(n);
}
```

### Growing, mutating, and other operations

Operations beyond literals, indexing, and iteration — appending, removing,
slicing out a sub-array, mapping, filtering, sorting — are standard-library
methods on `[T]`, not core syntax, and aren't finalized yet (see
[Introduction](../introduction.md)). The language guarantees `[T]` is
growable and mutable through any binding; the exact method surface is
future work.

## Tuples

A tuple groups a fixed number of values, possibly of different types, into
a single value, written `(T1, T2, ...)` as a type and `(e1, e2, ...)` as a
literal:

```roo
let point: (int, int) = (3, 4);
let mixed = (1, "two", 3.0); // (int, String, float), inferred
```

Tuple elements are accessed by position, with `.0`, `.1`, and so on:

```roo
let x = point.0;
let y = point.1;
```

Tuples can be destructured directly in a `let`, exactly like Rust:

```roo
let (x, y) = point;
```

### The unit type

`()`, the empty tuple, is roo's [unit type](overview.md#kinds-of-types) —
the type of an expression with no meaningful value, such as the value a
block produces when it has no trailing expression. Note that a function
with no `-> Type` in its signature is not the same as one annotated
`-> ()`: the return type is dynamic, not `()` — see
[Functions: Return type](../functions/functions.md#return-type).

### Tuples are reference types

Like `String` and `[T]`, a tuple is a compound type, not one of the four
primitives, so it follows reference semantics (see
[The Value Model](../design/values-and-mutation.md)): two bindings holding
"the same tuple" alias the same storage, and mutating an element through
one is visible through the other.

```roo
let a = (1, 2);
let b = a;   // b aliases the same tuple as a
b.0 = 99;
print(a.0);       // 99 — a and b share storage
```

