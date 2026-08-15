# The Value Model

Rust's rules for how values move between variables, function calls, and
struct fields are governed by ownership and borrowing, enforced by the borrow
checker. fig has neither, which means fig needs its own answer to a question
Rust answers with `&`, `&mut`, and `move`: **when you assign a value to a new
variable, or pass it to a function, do you get the same data or a copy of
it?**

fig answers this the way most garbage-collected scripting languages do —
Lua, Luau, JavaScript, Python — by splitting values into two kinds.

## Primitive types are values

`bool`, `int`, `float`, and `char` behave exactly like Rust's `Copy` types.
Assigning one, passing it to a function, or storing it in a field always
gives the receiver an independent value. There is no way to observe sharing
between two `int` bindings, because there is nothing to share — mutating one
can never affect the other.

```fig
let a: int = 1;
let b = a;      // b gets its own copy of the value 1
let c = a;
c += 1;
print(a); // 1 — unaffected by mutating c
```

## Everything else is a reference type

`String`, arrays, tuples, `struct`s, and `enum`s are all **reference
types**. A value of one of these types lives once, in memory the fig runtime
manages for you, and every variable, field, or function parameter that holds
"a `Point`" is holding a reference to that same, single `Point`. Assignment
and function calls copy the reference, not the data it points to — exactly
like assigning one JavaScript object variable to another, or one Lua table
variable to another.

```fig
struct Point { x: int, y: int }

let a = Point { x: 1, y: 2 };
let b = a;      // b refers to the same Point as a
b.x = 99;
print(a.x);          // 99 — a and b share one Point

fn bump(p: Point) {
    p.x += 1;        // mutates the caller's Point directly
}

bump(a);
print(a.x);           // 100
```

This is the single biggest semantic difference from idiomatic Rust: passing
a `struct` to a function in fig always shares the caller's data, the way
passing `&mut T` in Rust does — there is no by-value-vs-by-reference
*aliasing* choice to make, and no `&`/`&mut`/`.clone()` ceremony to get a
shared reference. Every binding — `let`, function/closure parameter, or
`self` — can always be reassigned and mutated through (see
[below](#every-binding-is-mutable)); `bump` above needed nothing beyond an
ordinary parameter to be allowed to write `p.x += 1`. If you need an
independent copy rather than a shared reference at all, ask for one
explicitly (see [Cloning](#cloning-explicit-copies) below); by default,
every reference type aliases.

## Why fig does this

Rust's alternative (move semantics, enforced by a borrow checker) exists to
let the compiler guarantee memory safety *without* a garbage collector or
runtime reference counting. fig's runtime manages memory for you, so that
guarantee isn't needed, and the ceremony that produces it (`&`, `&mut`,
lifetimes, move-then-use-after-move errors) has no job left to do. Reference
semantics for compound values is also simply how most embeddable scripting
languages behave — including Luau, the language fig is designed to
eventually replace in [fig-engine](https://github.com/LucaMezz/fig-engine) —
so scripts written against fig's object model should feel familiar to
scripters coming from that world.

## Every binding is mutable

Removing ownership also removes the reason for a separate `let` vs.
`let mut` distinction: there is no `mut` keyword in fig at all. Every
binding — `let`, function/closure parameter, or `self` — can always be
reassigned, and you can always mutate fields/elements through it:

```fig
let p = Point { x: 1, y: 2 };
p.x = 5;        // ok
p = Point { x: 0, y: 0 }; // ok — p can be reassigned entirely
```

Because reference types alias, two different bindings to the same
underlying value are always equally able to mutate it — there is no way for
one binding to a `Point` to be able to write through it while another
binding to that same `Point` cannot. Mutability isn't a property you choose
per binding; it's simply always available.

This is uniform across every kind of binding: a function parameter can
always mutate through it (as `bump` did above), and a method can always
mutate `self`'s fields — see [Functions](../functions/functions.md) and
[Structs: Methods](../data-types/structs.md#methods).

See [Variables](../bindings/variables.md) for the full rules on `let`.

## Equality compares values, not identity

`==` on two reference-typed values compares their *contents*
(structurally, field by field / element by element), the same way `==`
works on primitives — it does **not** check whether the two variables refer
to the same underlying object. Two separately-constructed `Point { x: 1, y: 2 }`
values are `==` to each other:

```fig
let a = Point { x: 1, y: 2 };
let b = Point { x: 1, y: 2 };
print(a == b); // true — same contents, even though a and b were built separately
```

## Cloning (explicit copies)

Because aliasing is the default, fig needs an explicit way to opt *out* of
it and get an independent copy. That facility is a standard-library concern
(analogous to Rust's `Clone` trait) and isn't finalized, but the language
guarantees every reference type can be explicitly copied on request — there
is no type that is aliasable-only. Until the standard library specifies the
exact mechanism, assume something in the shape of a `.clone()` method is
available on every reference type.

## Closures follow the same rule

A closure captures variables from its enclosing scope the same way a
function parameter receives an argument: primitives are captured by value,
reference types are captured by reference. There is no `move` keyword and no
`Fn`/`FnMut`/`FnOnce` distinction to choose between — see
[Closures](../functions/closures.md).
