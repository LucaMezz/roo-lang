# Result, Option, and the ? Operator

roo follows Rust's model for recoverable errors and absent values: they are
ordinary enum values you're required to handle, not exceptions and not a
`null` that silently inhabits every type. This chapter describes the
language-level contract; the concrete types are a standard-library concern
not finalized yet (see [Introduction](../introduction.md)) — everywhere
below, read `Option`/`Result` as "a type shaped like this," which the
standard library will provide.

## Representing absence: `Option`

A value that might not be present is represented by a two-variant generic
enum, conventionally shaped like:

```roo
enum Option<T> {
    Some(T),
    None,
}
```

rather than by a `null`/`nil` that silently inhabits every type. Because
`Option<T>` and `T` are different types, "forgetting" to handle the absent
case is a compile-time type error, not a runtime crash — you must pattern
match (or otherwise unwrap) an `Option<T>` to get at the `T` inside.

```roo
fn find(items: [int], target: int) -> Option<int> {
    for (index, item) in items {
        if item == target {
            return Option::Some(index);
        }
    }
    Option::None
}

match find(numbers, 5) {
    Option::Some(index) => print(index),
    Option::None => print("not found"),
}
```

## Representing recoverable errors: `Result`

A fallible operation returns a two-variant generic enum carrying either a
success value or an error value, conventionally shaped like:

```roo
enum Result<T, E> {
    Ok(T),
    Err(E),
}
```

```roo
fn parse(text: String) -> Result<int, String> {
    // ...
    Result::Err("invalid number")
}

match parse(input) {
    Result::Ok(n) => print(n),
    Result::Err(message) => print(message),
}
```

roo has no exceptions and no `try`/`catch` — every function that can fail
says so in its return type, and the compiler's exhaustiveness checking on
`match` (see [match](../control-flow/match.md)) ensures the failure case
can't be silently ignored the way an uncaught exception or an unchecked
error code can.

## The `?` operator

Matching on a `Result`/`Option` at every call site that can fail gets
repetitive when a function just wants to propagate the failure upward. `?`,
placed after an expression of `Result`/`Option` type, does exactly that: on
success it evaluates to the inner value; on failure it immediately returns
the failure out of the *current* function.

```roo
fn read_config(path: String) -> Result<Config, String> {
    let text = read_file(path)?;   // returns early with the Err if this fails
    let parsed = parse(text)?;      // same for this
    Result::Ok(parsed)
}
```

This is exactly equivalent to:

```roo
fn read_config(path: String) -> Result<Config, String> {
    let text = match read_file(path) {
        Result::Ok(value) => value,
        Result::Err(e) => return Result::Err(e),
    };
    let parsed = match parse(text) {
        Result::Ok(value) => value,
        Result::Err(e) => return Result::Err(e),
    };
    Result::Ok(parsed)
}
```

`?` can only be used inside a function whose own return type is compatible
with the failure it might propagate (a `Result` with a matching `Err` type,
or an `Option`) — using it in a function that returns something else is a
type error, exactly as in Rust.
