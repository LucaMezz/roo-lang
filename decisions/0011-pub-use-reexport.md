# 0011. `pub use` re-exports through the current module's path

**Status:** Accepted

## Context

Found while writing `examples/ecs.fig` (an earlier draft attempted it and
had to be corrected): `modules.md`'s `use` section never documented `pub
use`. Since items — including `use` imports — are private by default, a
plain `use` doesn't make the imported name reachable through the
importing module's own path. That was never confirmed either way, and
re-exporting (giving a module a flatter public path than its actual
internal structure) is a common, real need.

## Decision

`pub use path::Item;` re-exports the imported item through the current
module's own path, exactly matching Rust, with the same grouping/
renaming/glob forms plain `use` already supports.

## Rationale

No real alternative was considered — `pub use` behaving any other way
wouldn't match any existing precedent and wasn't in tension with anything
else fig had already decided. Documented as a straightforward addition,
not a design fork.

See: `book/src/modules/modules.md` (`use` section).
