//! Pirx core — discrete-event simulation engine for FTQC execution profiling.
//!
//! This crate contains the DES engine, factory models, injection error
//! model, trace collection, and post-hoc analysis. It depends on
//! [`pirx_ir`] for the circuit representation and [`pirx_hw`] for the
//! hardware model — nothing else.

pub mod analysis;
pub mod buffer;
pub mod dag;
pub mod engine;
pub mod events;
pub mod factory;
pub mod trace;
