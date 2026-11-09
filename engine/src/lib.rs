//! Tandem engine: mirror what Windows plays on one device (the source, typically the
//! slowest one, e.g. a Bluetooth amp) to any number of other outputs, each delayed and
//! clock-drift corrected so everything comes out in sync.

pub mod calibrate;
pub mod control;
pub mod devices;
pub mod engine;
pub mod mic;
pub mod mix;
pub mod sys;
