# Literals

## Integer literals

Decimal by default, with optional `_` digit separators anywhere between
digits:

```fig
let a = 42;
let b = 1_000_000;
```

Other bases use the same prefixes Rust does:

```fig
let hex = 0xFF;      // 255
let oct = 0o17;       // 15
let bin = 0b1010_1010; // 170
```

There is no suffix syntax (`42i32`, `10u8`) — fig has a single integer type,
[`int`](../types/primitives.md), so there is nothing for a suffix to select
between.

## Float literals

```fig
let pi = 3.14159;
let big = 1_000.0;
let short = 5.;      // trailing-dot form is allowed, like Rust
let exp = 1.5e10;
let neg_exp = 2.0e-3;
```

A bare integer-looking literal is never a `float` — `let x = 5;` gives you
an `int`. Write `5.0` (or annotate: `let x: float = 5;`) to get a `float`.

## Boolean literals

```fig
true
false
```

## Character literals

A `char` literal is a single Unicode scalar value in single quotes:

```fig
let c = 'a';
let newline = '\n';
let emoji = '🦀';
let quote = '\'';
```

## String literals

A `String` literal is UTF-8 text in double quotes, supporting the same
escape sequences as Rust:

```fig
let s = "hello, world";
let multiline = "line one
line two";
```

### Escape sequences

| Escape | Meaning |
|---|---|
| `\n` | newline |
| `\r` | carriage return |
| `\t` | tab |
| `\\` | backslash |
| `\0` | null character |
| `\"` | double quote |
| `\'` | single quote (also valid, though unnecessary, in a `"..."` string) |
| `\u{XXXX}` | Unicode scalar value by hex code point, e.g. `\u{1F980}` |

### Raw strings

A raw string ignores escape sequences entirely, which is useful for text
containing lots of backslashes or quotes (regular expressions, file paths,
embedded code):

```fig
let path = r"C:\Users\name";
let quoted = r#"she said "hello""#;
```

As in Rust, the number of `#` characters after `r` can be increased if the
string itself needs to contain `"#`.

## What's *not* here

Rust's numeric literal *type suffixes* (`1u8`, `2.0f32`, `3i64`) and byte
literals/byte strings (`b'a'`, `b"..."`) are tied to Rust's fixed-width
integer types and byte-buffer APIs, neither of which fig has — see
[Differences from Rust](../design/differences-from-rust.md). C-string
literals (`c"..."`) are FFI-only and likewise don't exist in fig.
