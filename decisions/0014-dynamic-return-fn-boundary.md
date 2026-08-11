# 0014. A dynamic-return function can fill a concrete `Fn` slot; checked at the call

**Status:** Accepted

## Context

A function or closure with no `-> Type` has a dynamically-typed return
(see [0006](0006-gradual-typing-inference-and-any.md)). Can such a
function be used wherever a concretely-typed `Fn(...) -> T` is expected —
e.g. stored in a `let handler: Fn(String) -> () = some_fn;` — and if so,
when does fig actually check that the function's real behavior matches
the promise?

## Decision

Yes, allowed, with no complaint at the point of assignment — the same
general gradual-typing boundary rule ordinary values already follow,
since a function/closure is just an ordinary value. The check happens
**at the call**, not wherever the caller later uses the returned value.

## Rationale

A function's return type is one more position that can be typed-or-
dynamic, the same as a struct field can be — nothing about functions
specifically needed a stricter, special-cased rule, and inventing one
would have cut against the "few special cases, uniform rules" pattern the
rest of the type system follows.

Pinning the check to the call, rather than deferring it to wherever the
result gets used, was the deliberate part: `()` results (and dynamic
values generally) are routinely discarded without ever being "used" for
anything. If the check were deferred that far, a call whose result goes
unused would silently skip verification entirely, and `Fn(...) -> T`
would stop being a promise the type system actually keeps. The call is
the one moment guaranteed to happen every time, mirroring where the
existing parameter-position boundary check already fires
(`takes_int(dynamic_value)` is checked at that call, not wherever `n`
gets used inside the function body).

`examples/dialogue.fig`'s `on_examine`/`on_use` closures already matched
this shape (dynamic-return closures stored in a `Fn(any) -> ()`-typed
slot) without anyone having engineered it that way.

See: `book/src/types/gradual-typing.md` ("The same boundary applies to a
function's return type").
