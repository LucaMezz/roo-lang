# Comments

fig has the same four comment forms Rust does.

## Line comments

`//` starts a comment that runs to the end of the line.

```fig
// This is a line comment.
let x = 1; // comments can trail code too
```

## Block comments

`/* ... */` comments out a span of text, and block comments **nest**, unlike
in C:

```fig
/* This is a block comment. */

/*
 * Block comments can span
 * multiple lines.
 */

/* outer /* inner */ still inside the outer comment */
```

## Doc comments

Doc comments attach documentation to the item that follows them, and are
written with three slashes instead of two:

```fig
/// Computes the Euclidean distance between two points.
fn distance(a: Point, b: Point) -> float {
    // ...
}
```

An inner doc comment, `//!`, documents the *enclosing* item (typically a
module or the whole file) rather than the item that follows it, and is
usually placed at the top of a file:

```fig
//! Geometry primitives used throughout the renderer.

struct Point { x: float, y: float }
```

Doc comments are ordinary comments as far as the language grammar is
concerned — the eventual documentation-generation tooling that gives them
meaning is a separate, not-yet-designed piece of tooling, the same
relationship `///` has to `rustdoc` in Rust.

## What's *not* here

Rust attributes like `#[doc = "..."]` (the desugared form of a doc comment)
are part of Rust's attribute/macro system, which fig doesn't have — see
[Differences from Rust](../design/differences-from-rust.md). `///` and `//!`
are ordinary lexical syntax in fig, not sugar for an attribute.
