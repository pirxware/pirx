//! Engine benchmarks — criterion + codspeed.
//!
//! Parameterized by circuit size to catch performance regressions
//! as the engine evolves. Run locally with `cargo bench` or in CI
//! via CodSpeed.

#![allow(clippy::unwrap_used)]

use codspeed_criterion_compat::{
    BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
};
use pirx_core::analysis::ProfileAnalyzer;
use pirx_core::engine::{Engine, EngineConfig};

const SEED: u64 = 42;

fn bench_engine_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine_new");
    group.sampling_mode(SamplingMode::Auto);

    for &size in &[10u32, 100, 500, 2000] {
        let circuit = pirx_testkit::t_gate_chain(size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &circuit, |b, circuit| {
            b.iter(|| {
                Engine::new(
                    circuit,
                    pirx_testkit::cultivation_hw(),
                    EngineConfig { seed: SEED },
                )
                .unwrap()
            });
        });
    }

    group.finish();
}

fn bench_engine_run(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine_run");
    group.sampling_mode(SamplingMode::Auto);

    for &size in &[10u32, 100, 500] {
        let circuit = pirx_testkit::t_gate_chain(size);
        // Warm start: pre-load buffer so the run isn't dominated by initial stall.
        let mut hw = pirx_testkit::cultivation_hw();
        hw.buffer.preload = 4;

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| {
                let engine =
                    Engine::new(&circuit, hw.clone(), EngineConfig { seed: SEED }).unwrap();
                engine.run()
            });
        });
    }

    group.finish();
}

fn bench_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("trace_analysis");
    group.sampling_mode(SamplingMode::Auto);

    for &size in &[10u32, 100, 500] {
        let circuit = pirx_testkit::t_gate_chain(size);
        let mut hw = pirx_testkit::cultivation_hw();
        hw.buffer.preload = 4;
        let trace = Engine::new(&circuit, hw, EngineConfig { seed: SEED })
            .unwrap()
            .run();

        group.bench_with_input(BenchmarkId::new("analyze", size), &trace, |b, trace| {
            b.iter(|| ProfileAnalyzer::analyze(trace, 1, 10));
        });
    }

    group.finish();
}

fn bench_engine_step(c: &mut Criterion) {
    let circuit = pirx_testkit::t_gate_chain(100);
    let mut hw = pirx_testkit::cultivation_hw();
    hw.buffer.preload = 4;

    c.bench_function("engine_step_single", |b| {
        b.iter_batched(
            || Engine::new(&circuit, hw.clone(), EngineConfig { seed: SEED }).unwrap(),
            |mut engine| engine.step(),
            codspeed_criterion_compat::BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    bench_engine_new,
    bench_engine_run,
    bench_analysis,
    bench_engine_step
);
criterion_main!(benches);
