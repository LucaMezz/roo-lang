//! One generated test per `examples/*.roo` file, each asserting it
//! parses with no error. See `build.rs` for how these are generated —
//! this file just pulls the generated source in.

use chumsky::Parser;

include!(concat!(env!("OUT_DIR"), "/example_tests.rs"));
