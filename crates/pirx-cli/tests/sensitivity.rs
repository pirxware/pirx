//! Integration tests for `pirx sensitivity morris`.

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use std::{path::PathBuf, process::Command};

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    fn pirx_bin() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_BIN_EXE_pirx"));
        assert!(path.exists(), "pirx binary not found at {}", path.display());
        path
    }

    #[test]
    fn cli_morris_runs() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("morris_result.json");

        let status = Command::new(pirx_bin())
            .args([
                "sensitivity",
                "morris",
                "--circuit",
                fixtures_dir()
                    .join("t_gate_chain_3.pirx.json")
                    .to_str()
                    .unwrap(),
                "--model",
                fixtures_dir().join("cultivation_hw.toml").to_str().unwrap(),
                "--sweep",
                fixtures_dir().join("sweep_morris.toml").to_str().unwrap(),
                "--output",
                output.to_str().unwrap(),
            ])
            .status()
            .unwrap();

        assert_eq!(
            status.code(),
            Some(0),
            "pirx sensitivity morris should exit 0"
        );
        assert!(output.exists(), "output file must exist");
    }

    #[test]
    fn cli_morris_output_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("morris_result.json");

        let status = Command::new(pirx_bin())
            .args([
                "sensitivity",
                "morris",
                "--circuit",
                fixtures_dir()
                    .join("t_gate_chain_3.pirx.json")
                    .to_str()
                    .unwrap(),
                "--model",
                fixtures_dir().join("cultivation_hw.toml").to_str().unwrap(),
                "--sweep",
                fixtures_dir().join("sweep_morris.toml").to_str().unwrap(),
                "--output",
                output.to_str().unwrap(),
            ])
            .status()
            .unwrap();

        assert_eq!(status.code(), Some(0));

        let json = std::fs::read_to_string(&output).unwrap();
        let result: pirx_sensitivity::MorrisResult =
            serde_json::from_str(&json).expect("output must be valid MorrisResult JSON");

        assert!(result.evaluations > 0);
        assert_eq!(result.parameters.len(), 2);
    }
}
