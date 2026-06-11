//! Pirx CLI — execution profiler for fault-tolerant quantum computing.

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
        /// Path to the circuit file.
        circuit: String,
        /// Path to the hardware model TOML file.
        #[arg(long)]
        hw: String,
        /// Output path for the JSON report.
        #[arg(long, default_value = "report.json")]
        output: String,
    },
}

fn main() {
    let _cli = Cli::parse();
    eprintln!("pirx: not yet implemented");
    std::process::exit(1);
}
