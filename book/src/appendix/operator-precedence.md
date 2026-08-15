# Operator Precedence

Operators are listed from highest to lowest precedence. Operators in the
same row group left-to-right unless noted otherwise.

| Precedence | Operator | Associativity |
|---|---|---|
| Highest | Paths (`::`) | left |
| | Method calls, field access (`.`) | left |
| | Function calls, indexing (`f()`, `a[i]`) | left |
| | `?` | — |
| | Unary `-`, unary `!` | — (prefix) |
| | `as` | left |
| | `*` `/` `%` | left |
| | `+` `-` | left |
| | `<<` `>>` | left |
| | `&` | left |
| | `^` | left |
| | <code>&#124;</code> | left |
| | `==` `!=` `<` `>` `<=` `>=` | requires parentheses to chain |
| | `&&` | left |
| | <code>&#124;&#124;</code> | left |
| | `..` `..=` | requires parentheses to chain |
| | `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` `<<=` `>>=` | right |
| Lowest | `return`, `break`, closures (`\|...\| ...`) | — |

## Notes

- Comparison operators (`==`, `!=`, `<`, `>`, `<=`, `>=`) don't chain: `a <
  b < c` is a parse error, not `(a < b) && (b < c)`. Write
  `a < b && b < c` explicitly.
- Range operators (`..`, `..=`) likewise don't chain and bind looser than
  the comparison/arithmetic operators above them but tighter than
  assignment: `0..x + 1` parses as `0..(x + 1)`.
- Parentheses always override precedence, exactly as expected: `(a + b) *
  c`.
- This table omits `&`/`&mut`/`*` as **prefix** (address-of/dereference)
  operators, since roo has no references — see
  [Operators](../expressions/operators.md#whats-not-here). `&`, `|`, and
  `*` above refer only to their binary (bitwise-and, bitwise-or,
  multiplication) forms.
