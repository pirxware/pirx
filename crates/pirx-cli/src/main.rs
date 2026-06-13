//! Pirx CLI — execution profiler for fault-tolerant quantum computing.

use std::path::{Path, PathBuf};

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "pirx",
    version,
    about = "Execution profiler for fault-tolerant quantum computing"
)]
enum Cli {
    /// Profile a circuit against a hardware model.
    Profile {
        /// Path to the circuit JSON file (.pirx.json).
        circuit: PathBuf,
        /// Path to the hardware model TOML file.
        #[arg(long)]
        hw: PathBuf,
        /// Output path for the JSON execution profile.
        #[arg(long, default_value = "report.json")]
        output: PathBuf,
        /// RNG seed for reproducible simulation. Default: 42.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Maximum simulation cycles. Omit for unbounded.
        #[arg(long)]
        max_cycles: Option<u64>,
        /// Analysis resolution in cycles per bucket. Default: 10.
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..))]
        resolution: u64,
    },

    /// Run Monte Carlo simulation: N replicas with different seeds.
    MonteCarlo {
        /// Path to the circuit JSON file (.pirx.json).
        circuit: PathBuf,
        /// Path to the hardware model TOML file.
        #[arg(long)]
        hw: PathBuf,
        /// Output path for the JSON Monte Carlo result.
        #[arg(long, default_value = "monte_carlo.json")]
        output: PathBuf,
        /// Number of independent replicas.
        #[arg(long, default_value_t = 100)]
        replicas: u32,
        /// Base RNG seed. Replica i uses seed + i.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Maximum simulation cycles per replica. Omit for unbounded.
        #[arg(long)]
        max_cycles: Option<u64>,
        /// Number of rayon threads. Omit for default (= num CPUs).
        #[arg(long)]
        threads: Option<usize>,
    },
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("circuit JSON error: {0}")]
    CircuitJson(#[from] serde_json::Error),
    #[error("circuit validation error: {0}")]
    Validation(#[from] pirx_ir::validate::ValidationError),
    #[error("hardware model error: {0}")]
    Hardware(#[from] pirx_hw::HardwareModelError),
    #[error("engine error: {0}")]
    Engine(#[from] pirx_core::EngineError),
    #[error("monte carlo error: {0}")]
    MonteCarlo(#[from] pirx_core::MonteCarloError),
}

impl CliError {
    fn exit_code(&self) -> i32 {
        match self {
            Self::Io(_) => 2,
            Self::CircuitJson(_)
            | Self::Validation(_)
            | Self::Hardware(_)
            | Self::Engine(_)
            | Self::MonteCarlo(_) => 1,
        }
    }
}

fn run_profile(
    circuit: &Path,
    hw: &Path,
    output: &Path,
    seed: u64,
    max_cycles: Option<u64>,
    resolution: u64,
) -> Result<(), CliError> {
    let circuit_json = std::fs::read_to_string(circuit)?;
    let profiler_circuit: pirx_ir::circuit::ProfilerCircuit = serde_json::from_str(&circuit_json)?;
    let validated = pirx_ir::validate::validate(profiler_circuit)?;

    let hw_toml = std::fs::read_to_string(hw)?;
    let hw_model = pirx_hw::model::load(&hw_toml)?;

    let factory_count = hw_model.factory.count();
    let config = pirx_core::EngineConfig { seed, max_cycles };
    let engine = pirx_core::Engine::new(&validated, &hw_model, config)?;
    let trace = engine.run();

    #[allow(clippy::cast_possible_truncation)]
    let factory_count_u16 = factory_count.min(u32::from(u16::MAX)) as u16;
    let profile = pirx_core::ProfileAnalyzer::analyze(&trace, factory_count_u16, resolution);

    let json = serde_json::to_string_pretty(&profile)?;
    std::fs::write(output, &json)?;

    eprintln!(
        "pirx: {} ops, {} cycles, {} stalls, {} fixups, infidelity={:.2e} → {}",
        validated.ops.len(),
        trace.total_cycles,
        profile.stall_events.len(),
        profile.fixups_inserted,
        profile.total_infidelity,
        output.display(),
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_monte_carlo_cmd(
    circuit: &Path,
    hw: &Path,
    output: &Path,
    replicas: u32,
    seed: u64,
    max_cycles: Option<u64>,
    threads: Option<usize>,
) -> Result<(), CliError> {
    let circuit_json = std::fs::read_to_string(circuit)?;
    let profiler_circuit: pirx_ir::circuit::ProfilerCircuit = serde_json::from_str(&circuit_json)?;
    let validated = pirx_ir::validate::validate(profiler_circuit)?;

    let hw_toml = std::fs::read_to_string(hw)?;
    let hw_model = pirx_hw::model::load(&hw_toml)?;

    let mc_config = pirx_core::MonteCarloConfig {
        replicas,
        base_seed: seed,
        max_cycles,
        threads,
    };

    let start = std::time::Instant::now();
    let result = pirx_core::run_monte_carlo(&validated, &hw_model, mc_config)?;
    let elapsed = start.elapsed();

    let json = serde_json::to_string_pretty(&result)?;
    std::fs::write(output, &json)?;

    let rps = if elapsed.as_secs_f64() > 0.0 {
        f64::from(replicas) / elapsed.as_secs_f64()
    } else {
        0.0
    };

    eprintln!(
        "Monte Carlo: {} replicas, seed {}, {} threads",
        replicas,
        seed,
        threads.map_or("default".to_owned(), |t| t.to_string()),
    );
    eprintln!(
        "Runtime (cycles): mean={:.0} \u{00b1} {:.0}, median={:.0}, p95={:.0}",
        result.total_cycles.mean,
        result.total_cycles.stddev,
        result.total_cycles.median,
        result.total_cycles.p95,
    );
    eprintln!(
        "Stalls: mean={:.1} \u{00b1} {:.1}, median={:.0}, p95={:.0}",
        result.stall_count.mean,
        result.stall_count.stddev,
        result.stall_count.median,
        result.stall_count.p95,
    );
    eprintln!(
        "Max stall (cycles): mean={:.0} \u{00b1} {:.0}, p95={:.0}",
        result.max_stall_cycles.mean, result.max_stall_cycles.stddev, result.max_stall_cycles.p95,
    );
    eprintln!(
        "Injection errors: mean={:.1} \u{00b1} {:.1}",
        result.injection_errors.mean, result.injection_errors.stddev,
    );
    eprintln!(
        "Factory utilization: mean={:.2} \u{00b1} {:.2}",
        result.mean_factory_utilization.mean, result.mean_factory_utilization.stddev,
    );
    eprintln!(
        "Buffer full: mean={:.1} \u{00b1} {:.1}",
        result.buffer_full_events.mean, result.buffer_full_events.stddev,
    );
    eprintln!(
        "Infidelity: mean={:.2e} \u{00b1} {:.2e}, p95={:.2e}",
        result.total_infidelity.mean, result.total_infidelity.stddev, result.total_infidelity.p95,
    );
    eprintln!("Truncated: {}/{}", result.truncated_count, replicas,);
    eprintln!(
        "Wall time: {:.1}s ({:.1} replicas/s) \u{2192} {}",
        elapsed.as_secs_f64(),
        rps,
        output.display(),
    );

    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let result = match cli {
        Cli::Profile {
            circuit,
            hw,
            output,
            seed,
            max_cycles,
            resolution,
        } => run_profile(&circuit, &hw, &output, seed, max_cycles, resolution),
        Cli::MonteCarlo {
            circuit,
            hw,
            output,
            replicas,
            seed,
            max_cycles,
            threads,
        } => run_monte_carlo_cmd(&circuit, &hw, &output, replicas, seed, max_cycles, threads),
    };
    if let Err(e) = result {
        let code = e.exit_code();
        eprintln!("pirx: {e}");
        std::process::exit(code);
    }
}
