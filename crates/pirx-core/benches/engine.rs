//! Engine benchmarks — criterion + codspeed.

use codspeed_criterion_compat::{Criterion, criterion_group, criterion_main};

fn bench_placeholder(c: &mut Criterion) {
    c.bench_function("placeholder", |b| {
        b.iter(|| {
            // TODO: benchmark Engine::new + engine.run on a small circuit
            std::hint::black_box(42)
        });
    });
}

criterion_group!(benches, bench_placeholder);
criterion_main!(benches);
