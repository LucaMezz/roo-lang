# Strings and Characters

## `String`

fig has one string type, `String`: UTF-8 encoded text, growable, and — like
every non-primitive type — a **reference type** (see
[The Value Model](../design/values-and-mutation.md)). String literals
produce `String` values directly.

```fig
let name: String = "Ada";
let mut greeting = "Hello, ";
greeting += name; // string concatenation with +=
```

Rust splits text into two types: `str`, an unsized, usually-borrowed view
into UTF-8 bytes, and `String`, an owned, growable buffer — a split that
exists to let a borrowed function parameter (`&str`) accept both a literal
and an owned `String` without copying, under the borrow checker's rules.
Since fig has neither borrowing nor a distinction between owned and
borrowed data, that split collapses to one type. Every fig string, whether
it came from a literal or was built at runtime, is a `String`, and passing
one to a function never copies its contents — see
[Differences from Rust](../design/differences-from-rust.md).

### Indexing

Because `String` is UTF-8 and a `char` can be encoded in more than one
byte, `String` does not support integer indexing (`s[0]`) the same way an
array does — this matches Rust, which also disallows `s[0]` on `str`/
`String` for the same reason. Character-level and substring access are
standard-library concerns (not yet designed) rather than core language
syntax.

### Comparison

`String`s compare by content with `==`/`!=`, and order lexicographically
with `<`/`<=`/`>`/`>=`, exactly like Rust `String`/`str` — see
[Equality compares values, not identity](../design/values-and-mutation.md#equality-compares-values-not-identity).

## `char`

See [`char` under Primitive Types](primitives.md#char). A `char` is a value
type, not a reference type — copied on assignment, distinct from
`String`, and not implicitly interchangeable with a one-character string.

```fig
let c: char = 'A';
let s: String = "A"; // a String, not a char — these are different types
```
