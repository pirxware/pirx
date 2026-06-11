//! Pirx core — discrete-event simulation engine for FTQC execution profiling.
//!
//! This crate contains the DES engine, factory models, injection error
//! model, trace collection, and post-hoc analysis. It depends on
//! [`pirx_ir`] for the circuit representation and [`pirx_hw`] for the
//! hardware model — nothing else.

pub mod trace;

// TODO: engine, dag, factory models, buffer, metrics, analysis
