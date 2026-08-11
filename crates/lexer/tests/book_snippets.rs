//! One generated test per fig code block in the book, plus one per
//! `examples/*.fig` file, each asserting it lexes with no error. See
//! `build.rs` for how these are generated — this file just pulls the
//! generated source in.

include!(concat!(env!("OUT_DIR"), "/book_snippets_tests.rs"));
