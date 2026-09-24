//! CADBench: an eval harness for typed-decision-driven CAD design agents.
//!
//! Backend-agnostic by design — see [`runner`] for why — so this crate never
//! depends on the thing it measures. `transmog` is the first [`runner::Backend`];
//! zoo.dev/KittyCAD's text-to-CAD API is the named next one.

pub mod lua_check;
pub mod runner;
pub mod scorer;
mod step_oracle;
pub mod task;
