//! Tandem engine: mirror what Windows plays on one device (the source, typically the
//! slowest one, e.g. a Bluetooth amp) to any number of other outputs, each delayed and
//! clock-drift corrected so everything comes out in sync.

// `unsafe` is confined to the two modules that call Windows directly, `sys` and `sessions`,
// which opt in at the top of their file and say why. Everything else is checked by the
// compiler.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod calibrate;
pub mod control;
pub mod devices;
pub mod engine;
pub mod mic;
pub mod mix;
pub mod sessions;
pub mod sys;
