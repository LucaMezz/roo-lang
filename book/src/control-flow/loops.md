# Loops

roo has the same three loop forms as Rust: `loop`, `while`, and `for`, plus
`while let`.

## `loop`

An unconditional loop that runs until a `break`:

```roo
let count = 0;
loop {
    count += 1;
    if count == 5 {
        break;
    }
}
```

### `loop` as an expression

`break` can carry a value, making `loop` the only loop form that can
evaluate to something other than `()` — useful for retry-until-success
logic:

```roo
let attempts = 0;
let result = loop {
    attempts += 1;
    if let Some(value) = try_connect() {
        break value;
    }
    if attempts > 3 {
        break -1;
    }
};
```

## `while`

Runs its body as long as a `bool` condition holds, checked before each
iteration:

```roo
let n = 10;
while n > 0 {
    print(n);
    n -= 1;
}
```

## `while let`

Like `if let`, but loops as long as the pattern keeps matching:

```roo
while let Some(item) = queue.pop() {
    process(item);
}
```

## `for`

Iterates over every element produced by an iterable expression, binding
each in turn:

```roo
for n in [1, 2, 3] {
    print(n);
}

for i in 0..10 {      // exclusive range: 0 through 9
    print(i);
}

for i in 0..=10 {     // inclusive range: 0 through 10
    print(i);
}
```

`for` works over arrays, ranges, and — in general — anything that
implements the iteration protocol (analogous to Rust's `Iterator` trait).
The exact shape of that protocol is a standard-library concern not yet
finalized (see [Introduction](../introduction.md)); the language guarantees
that `for pattern in expr { }` desugars to repeatedly producing a value from
`expr` and matching it against `pattern`, exactly as in Rust.

The loop variable can be any pattern, not just a single name — see
[Pattern Matching](../data-types/pattern-matching.md):

```roo
for (index, name) in enumerated_names {
    print(index);
    print(name);
}
```

## `break` and `continue`

`break` exits the innermost loop immediately; `continue` skips to the next
iteration:

```roo
for n in 0..100 {
    if n % 2 != 0 {
        continue;
    }
    if n > 10 {
        break;
    }
    print(n);
}
```

## Labeled loops

When loops are nested, a label lets `break`/`continue` target an outer
loop instead of the innermost one. A label is a plain identifier followed
by a colon, placed before the loop keyword, and referenced by name (not by
prefixing it with `'`) in `break`/`continue`:

```roo
outer: for x in 0..5 {
    for y in 0..5 {
        if x * y > 6 {
            break outer;
        }
        if y > x {
            continue outer;
        }
        print(x * y);
    }
}
```

Rust spells a loop label `'outer:`, reusing the same leading-apostrophe
syntax as a lifetime, because in Rust a label technically *is* a lifetime
occupying the label namespace. Since roo has no lifetimes at all, its loop
labels are spelled as plain identifiers instead, so the syntax doesn't imply
a feature that isn't there — see
[Differences from Rust](../design/differences-from-rust.md).

### Labels vs. values named the same thing

Plain-identifier labels create one small wrinkle Rust's `'outer` spelling
doesn't have: `break x;` could mean "break the loop labeled `x`" or "break
the innermost loop with the value of variable `x`," and nothing about the
syntax itself tells them apart. The rule is simple and deterministic: **if
the identifier right after `break`/`continue` names an in-scope loop
label, it's always the label**, never a variable read — the same way an
inner `let` shadows an outer one of the same name, just applied to labels
and values sharing a namespace at that one spot. A variable that happens
to share a name with an enclosing label is simply not reachable as a
`break`/`continue` value from inside that loop; rename one or the other if
this ever actually comes up, which in practice should be rare — a loop
label and a nearby variable sharing a name is already confusing style
independent of this rule.
