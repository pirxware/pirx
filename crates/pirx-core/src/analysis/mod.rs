//! Post-hoc trace analysis — single O(n) pass over raw [`TraceEvent`]s.
//!
//! [`ProfileAnalyzer::analyze`] reads a [`Trace`] produced by the engine and
//! returns a time-bucketed [`ExecutionProfile`]. No engine state is touched;
//! the analyzer is a pure function of the trace.

mod analyzer;
mod profile;

pub use analyzer::ProfileAnalyzer;
pub use profile::{BottleneckType, ExecutionProfile, StallRecord};
