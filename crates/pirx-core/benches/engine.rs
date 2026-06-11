//! Engine benchmarks — criterion + codspeed.
//!
//! Real benchmarks land with the engine implementation.
//! Empty group registers no data points — CodSpeed starts clean.

use codspeed_criterion_compat::{Criterion, criterion_group, criterion_main};

fn benchmarks(_c: &mut Criterion) {}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
