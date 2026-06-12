//! Pirx CLI — execution profiler for fault-tolerant quantum computing.

use std::path::PathBuf;

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
        #[arg(long, default_value_t = 10)]
        resolution: u64,
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
}

impl CliError {
    fn exit_code(&self) -> i32 {
        match self {
            Self::Io(_) => 2,
            Self::CircuitJson(_) | Self::Validation(_) | Self::Hardware(_) | Self::Engine(_) => 1,
        }
    }
}

fn run_profile(
    circuit: &PathBuf,
    hw: &PathBuf,
    output: &PathBuf,
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
        "pirx: {} ops, {} cycles, {} stalls, {} fixups → {}",
        validated.ops.len(),
        trace.total_cycles,
        profile.stall_events.len(),
        profile.fixups_inserted,
        output.display(),
    );

    Ok(())
}

fn main() {
    let cli = Cli::parse();
    match cli {
        Cli::Profile {
            circuit,
            hw,
            output,
            seed,
            max_cycles,
            resolution,
        } => {
            if let Err(e) = run_profile(&circuit, &hw, &output, seed, max_cycles, resolution) {
                let code = e.exit_code();
                eprintln!("pirx: {e}");
                std::process::exit(code);
            }
        }
    }
}
