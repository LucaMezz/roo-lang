## Summary

<!-- What does this change, and why? -->

## Related issue

<!-- Closes #... , or "none" -->

## Checklist

- [ ] Commit messages follow Conventional Commits, using a scope from `cog.toml` (`cog check` passes locally)
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --workspace --all-targets` passes with no new warnings
- [ ] `cargo test --workspace` passes
- [ ] Tests were added or updated for the change
- [ ] The book (`book/src`) was updated if language behavior changed
