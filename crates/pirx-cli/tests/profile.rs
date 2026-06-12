//! Integration tests for the `pirx profile` subcommand.

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use std::{path::PathBuf, process::Command};

    use pirx_core::ExecutionProfile;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    fn pirx_bin() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_BIN_EXE_pirx"));
        assert!(path.exists(), "pirx binary not found at {}", path.display());
        path
    }

    #[test]
    fn profile_produces_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("report.json");

        let status = Command::new(pirx_bin())
            .args([
                "profile",
                fixtures_dir()
                    .join("t_gate_chain_3.pirx.json")
                    .to_str()
                    .unwrap(),
                "--hw",
                fixtures_dir().join("cultivation_hw.toml").to_str().unwrap(),
                "--output",
                output.to_str().unwrap(),
                "--seed",
                "42",
                "--resolution",
                "10",
            ])
            .status()
            .unwrap();

        assert_eq!(status.code(), Some(0), "pirx profile should exit 0");
        assert!(output.exists(), "output file must exist");

        let json = std::fs::read_to_string(&output).unwrap();
        let profile: ExecutionProfile =
            serde_json::from_str(&json).expect("output must be valid ExecutionProfile JSON");

        assert!(profile.total_cycles > 0);
        assert_eq!(profile.resolution, 10);
    }

    #[test]
    fn profile_deterministic_across_runs() {
        let dir = tempfile::tempdir().unwrap();

        let mut reports = Vec::new();
        for i in 0..2 {
            let output = dir.path().join(format!("report_{i}.json"));
            let status = Command::new(pirx_bin())
                .args([
                    "profile",
                    fixtures_dir()
                        .join("t_gate_chain_3.pirx.json")
                        .to_str()
                        .unwrap(),
                    "--hw",
                    fixtures_dir().join("cultivation_hw.toml").to_str().unwrap(),
                    "--output",
                    output.to_str().unwrap(),
                    "--seed",
                    "42",
                ])
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(0));
            reports.push(std::fs::read_to_string(&output).unwrap());
        }

        assert_eq!(
            reports[0], reports[1],
            "same seed must produce identical output"
        );
    }

    #[test]
    fn profile_missing_circuit_exits_2() {
        let status = Command::new(pirx_bin())
            .args([
                "profile",
                "/nonexistent/circuit.pirx.json",
                "--hw",
                fixtures_dir().join("cultivation_hw.toml").to_str().unwrap(),
            ])
            .status()
            .unwrap();

        assert_eq!(
            status.code(),
            Some(2),
            "missing file should exit 2 (I/O error)"
        );
    }

    #[test]
    fn profile_invalid_circuit_json_exits_1() {
        let dir = tempfile::tempdir().unwrap();
        let bad_circuit = dir.path().join("bad.pirx.json");
        std::fs::write(&bad_circuit, "{ not valid json !!!").unwrap();

        let status = Command::new(pirx_bin())
            .args([
                "profile",
                bad_circuit.to_str().unwrap(),
                "--hw",
                fixtures_dir().join("cultivation_hw.toml").to_str().unwrap(),
            ])
            .status()
            .unwrap();

        assert_eq!(status.code(), Some(1), "invalid JSON should exit 1");
    }

    #[test]
    fn profile_invalid_hw_toml_exits_1() {
        let dir = tempfile::tempdir().unwrap();
        let bad_hw = dir.path().join("bad.toml");
        std::fs::write(&bad_hw, "not = [valid toml for hw").unwrap();

        let status = Command::new(pirx_bin())
            .args([
                "profile",
                fixtures_dir()
                    .join("t_gate_chain_3.pirx.json")
                    .to_str()
                    .unwrap(),
                "--hw",
                bad_hw.to_str().unwrap(),
            ])
            .status()
            .unwrap();

        assert_eq!(status.code(), Some(1), "invalid HW TOML should exit 1");
    }

    #[test]
    fn profile_max_cycles_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("report.json");

        let status = Command::new(pirx_bin())
            .args([
                "profile",
                fixtures_dir()
                    .join("t_gate_chain_3.pirx.json")
                    .to_str()
                    .unwrap(),
                "--hw",
                fixtures_dir().join("cultivation_hw.toml").to_str().unwrap(),
                "--output",
                output.to_str().unwrap(),
                "--seed",
                "42",
                "--max-cycles",
                "1",
            ])
            .status()
            .unwrap();

        assert_eq!(status.code(), Some(0));
        let json = std::fs::read_to_string(&output).unwrap();
        let profile: ExecutionProfile = serde_json::from_str(&json).unwrap();
        assert!(
            profile.total_cycles <= 1,
            "max_cycles=1 should truncate simulation"
        );
    }

    #[test]
    fn profile_stderr_summary() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("report.json");

        let result = Command::new(pirx_bin())
            .args([
                "profile",
                fixtures_dir()
                    .join("t_gate_chain_3.pirx.json")
                    .to_str()
                    .unwrap(),
                "--hw",
                fixtures_dir().join("cultivation_hw.toml").to_str().unwrap(),
                "--output",
                output.to_str().unwrap(),
                "--seed",
                "42",
            ])
            .output()
            .unwrap();

        assert!(result.status.success());
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.starts_with("pirx:"),
            "stderr should start with 'pirx:' summary, got: {stderr}"
        );
        assert!(
            stderr.contains("ops"),
            "stderr summary should mention ops count"
        );
    }
}
