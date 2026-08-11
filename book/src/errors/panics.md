# Panics

Not every failure is meant to be recoverable. A **panic** is fig's
mechanism for an unrecoverable error — a bug, or a violated invariant,
where the correct response is to stop rather than to keep running with bad
state.

```fig
fn divide(a: int, b: int) -> int {
    if b == 0 {
        panic("division by zero");
    }
    a / b
}
```

Operations that have no sensible result also panic rather than returning a
special value: indexing an array out of bounds, for instance (see
[Arrays and Tuples](../types/arrays-and-tuples.md#indexing)), panics rather
than returning `None` or a garbage value.

## Panics vs. `Result`

The rule of thumb is the same as Rust's: use `Result` for failures a caller
can reasonably anticipate and recover from (a missing file, invalid user
input); reserve panics for conditions that indicate a bug (an index that
should never be out of range if the code above it is correct, an invariant
your own code is supposed to guarantee). A library function generally
shouldn't panic on bad *input* — it should return a `Result` and let the
caller decide what to do.

## What happens when a panic occurs

The precise runtime behavior of a panic — whether it unwinds the current
call stack, running cleanup code as it goes, or aborts the program
immediately, and how a host embedding fig (like fig-engine) can intercept
one — is a runtime concern, not language syntax, and isn't finalized yet.
What's guaranteed at the language level is that a panic immediately stops
normal execution of the panicking function and everything it called,
propagating upward; it is not a value that can be matched on or ignored the
way a `Result::Err` can.
