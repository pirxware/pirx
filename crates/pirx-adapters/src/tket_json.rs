//! tket1 JSON adapter — converts tket1 JSON circuit files into Profiler IR.

use std::{collections::HashMap, path::Path};

use pirx_ir::{
    circuit::{
        CircuitMetadata, Dependency, OpId, OpKind, Operation, ProfilerCircuit, QubitId,
        classify_rz_angle,
    },
    validate::ValidatedCircuit,
};
use smallvec::SmallVec;
use tket_json_rs::{SerialCircuit, optype::OpType, register::ElementId};

use crate::error::TketJsonError;

/// Parse a tket1 JSON file and return a validated profiler circuit.
#[must_use = "parsing a circuit without using the result is always a bug"]
pub fn from_tket_json_file(path: &Path) -> Result<ValidatedCircuit, TketJsonError> {
    let content = std::fs::read_to_string(path)?;
    from_tket_json_str(&content)
}

/// Parse a tket1 JSON string and return a validated profiler circuit.
#[must_use = "parsing a circuit without using the result is always a bug"]
pub fn from_tket_json_str(json: &str) -> Result<ValidatedCircuit, TketJsonError> {
    let serial: SerialCircuit =
        serde_json::from_str(json).map_err(|e| TketJsonError::Parse(e.to_string()))?;
    build_circuit(&serial)
}

// ── Circuit builder ────────────────────────────────────────────────────────

struct CircuitBuilder {
    qubit_map: HashMap<ElementId, QubitId>,
    qubit_count: u32,
    ops: Vec<Operation>,
    deps: Vec<Dependency>,
    last_on_qubit: Vec<Option<OpId>>,
    next_id: OpId,
    t_count: u64,
    clifford_count: u64,
    rotation_count: u64,
    depth_at_qubit: Vec<u64>,
}

impl CircuitBuilder {
    fn new(serial: &SerialCircuit) -> Result<Self, TketJsonError> {
        let qubit_count = u32::try_from(serial.qubits.len()).map_err(|_| {
            TketJsonError::Parse(format!(
                "qubit count {} exceeds u32 maximum",
                serial.qubits.len()
            ))
        })?;
        let mut qubit_map = HashMap::with_capacity(serial.qubits.len());
        for (idx, qubit) in serial.qubits.iter().enumerate() {
            // idx < serial.qubits.len() which fits in u32 (checked above)
            #[allow(clippy::cast_possible_truncation)]
            qubit_map.insert(qubit.id.clone(), idx as QubitId);
        }

        Ok(Self {
            qubit_map,
            qubit_count,
            ops: Vec::new(),
            deps: Vec::new(),
            last_on_qubit: vec![None; qubit_count as usize],
            next_id: 0,
            t_count: 0,
            clifford_count: 0,
            rotation_count: 0,
            depth_at_qubit: vec![0; qubit_count as usize],
        })
    }

    fn process_commands(&mut self, serial: &SerialCircuit) -> Result<(), TketJsonError> {
        for cmd in &serial.commands {
            let op_type = cmd.op.op_type;

            if is_structural(op_type) {
                continue;
            }

            reject_unsupported(op_type)?;

            let qubit_ids = self.resolve_qubit_args(&cmd.args);

            if qubit_ids.is_empty() {
                continue;
            }

            let kind = classify_op(op_type, &cmd.op.params)?;

            self.emit_operation(kind, &qubit_ids);
        }
        Ok(())
    }

    fn resolve_qubit_args(&self, args: &[ElementId]) -> SmallVec<[QubitId; 2]> {
        let mut qubit_ids = SmallVec::new();
        for arg in args {
            if let Some(&qubit_id) = self.qubit_map.get(arg) {
                qubit_ids.push(qubit_id);
            }
        }
        qubit_ids
    }

    fn emit_operation(&mut self, kind: OpKind, qubits: &[QubitId]) {
        let id = self.next_id;

        let op = Operation {
            id,
            kind,
            qubits: SmallVec::from_slice(qubits),
            initially_active: true,
        };

        for &q in qubits {
            if let Some(prev_id) = self.last_on_qubit.get(q as usize).copied().flatten() {
                self.deps.push(Dependency {
                    from: prev_id,
                    to: id,
                });
            }
            if let Some(slot) = self.last_on_qubit.get_mut(q as usize) {
                *slot = Some(id);
            }
        }

        let op_depth = qubits
            .iter()
            .filter_map(|&q| self.depth_at_qubit.get(q as usize).copied())
            .max()
            .unwrap_or(0)
            + 1;
        for &q in qubits {
            if let Some(d) = self.depth_at_qubit.get_mut(q as usize) {
                *d = op_depth;
            }
        }

        match kind {
            OpKind::TGate => self.t_count += 1,
            OpKind::Clifford => self.clifford_count += 1,
            OpKind::Rotation { .. } => self.rotation_count += 1,
            OpKind::Measurement { .. } => {}
        }

        self.ops.push(op);
        self.next_id += 1;
    }

    fn finish(self, name: Option<&str>) -> Result<ValidatedCircuit, TketJsonError> {
        let max_depth = self.depth_at_qubit.iter().copied().max().unwrap_or(0);
        let circuit = ProfilerCircuit {
            ops: self.ops,
            deps: self.deps,
            qubit_count: self.qubit_count,
            qubit_positions: None,
            hooks: vec![],
            metadata: CircuitMetadata {
                name: name.unwrap_or_default().to_string(),
                source_framework: "tket".into(),
                t_count: self.t_count,
                clifford_count: self.clifford_count,
                rotation_count: self.rotation_count,
                depth: max_depth,
            },
        };
        pirx_ir::validate::validate(circuit).map_err(TketJsonError::from)
    }
}

fn build_circuit(serial: &SerialCircuit) -> Result<ValidatedCircuit, TketJsonError> {
    let mut builder = CircuitBuilder::new(serial)?;
    builder.process_commands(serial)?;
    builder.finish(serial.name.as_deref())
}

// ── Operation classification ───────────────────────────────────────────────

fn is_structural(op_type: OpType) -> bool {
    matches!(
        op_type,
        OpType::Input
            | OpType::Output
            | OpType::Create
            | OpType::Discard
            | OpType::ClInput
            | OpType::ClOutput
            | OpType::Barrier
            | OpType::Label
            | OpType::Branch
            | OpType::Goto
            | OpType::Stop
            | OpType::noop
    )
}

fn reject_unsupported(op_type: OpType) -> Result<(), TketJsonError> {
    if is_box(op_type) || is_conditional(op_type) || is_classical(op_type) {
        return Err(TketJsonError::UnsupportedOperation {
            op_type: op_type.to_string(),
        });
    }
    Ok(())
}

fn is_box(op_type: OpType) -> bool {
    matches!(
        op_type,
        OpType::CircBox
            | OpType::Unitary1qBox
            | OpType::Unitary2qBox
            | OpType::Unitary3qBox
            | OpType::ExpBox
            | OpType::PauliExpBox
            | OpType::PauliExpPairBox
            | OpType::PauliExpCommutingSetBox
            | OpType::TermSequenceBox
            | OpType::CliffBox
            | OpType::PhasePolyBox
            | OpType::StabiliserAssertionBox
            | OpType::ProjectorAssertionBox
            | OpType::QControlBox
            | OpType::UnitaryTableauBox
            | OpType::ClassicalExpBox
            | OpType::MultiplexorBox
            | OpType::MultiplexedRotationBox
            | OpType::MultiplexedU2Box
            | OpType::MultiplexedTensoredU2Box
            | OpType::ToffoliBox
            | OpType::ConjugationBox
            | OpType::DummyBox
            | OpType::StatePreparationBox
            | OpType::DiagonalBox
            | OpType::CustomGate
    )
}

fn is_conditional(op_type: OpType) -> bool {
    matches!(op_type, OpType::Conditional)
}

fn is_classical(op_type: OpType) -> bool {
    matches!(
        op_type,
        OpType::ClassicalTransform
            | OpType::WASM
            | OpType::SetBits
            | OpType::CopyBits
            | OpType::RangePredicate
            | OpType::ExplicitPredicate
            | OpType::ExplicitModifier
            | OpType::MultiBit
            | OpType::ClExpr
    )
}

fn classify_op(op_type: OpType, params: &Option<Vec<String>>) -> Result<OpKind, TketJsonError> {
    match op_type {
        OpType::T | OpType::Tdg => Ok(OpKind::TGate),
        OpType::Measure | OpType::Collapse | OpType::Reset => {
            Ok(OpKind::Measurement { hook: None })
        }
        OpType::Rz | OpType::Rx | OpType::Ry | OpType::U1 | OpType::Phase => {
            let op_type_name = op_type.to_string();
            let angle_halfturns = parse_first_param(params, &op_type_name)?;
            let angle_radians = angle_halfturns * std::f64::consts::PI;
            Ok(classify_rz_angle(angle_radians))
        }
        _ => Ok(OpKind::Clifford),
    }
}

fn parse_first_param(
    params: &Option<Vec<String>>,
    op_type_name: &str,
) -> Result<f64, TketJsonError> {
    let params = params.as_ref().ok_or_else(|| {
        TketJsonError::Parse(format!(
            "gate '{op_type_name}' requires a parameter but has none"
        ))
    })?;

    let param_str = params.first().ok_or_else(|| {
        TketJsonError::Parse(format!(
            "gate '{op_type_name}' requires a parameter but has none"
        ))
    })?;

    param_str
        .parse::<f64>()
        .map_err(|_| TketJsonError::SymbolicParameter {
            gate: op_type_name.into(),
            // clone: need owned String for error variant; param_str borrows from SerialCircuit
            param: param_str.clone(),
        })
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn load_small_circuit_from_file() {
        let circuit = from_tket_json_file(&fixture_path("small_circuit.json")).unwrap();
        assert_eq!(circuit.qubit_count, 3);
        assert_eq!(circuit.metadata.source_framework, "tket");
    }

    #[test]
    fn gate_classification() {
        let circuit = from_tket_json_file(&fixture_path("small_circuit.json")).unwrap();

        // h, cx → Clifford; t, tdg, rz(0.25) → TGate; rz(0.3) → Rotation; s → Clifford; 3× measure
        assert_eq!(circuit.metadata.t_count, 3);
        assert_eq!(circuit.metadata.clifford_count, 3);
        assert_eq!(circuit.metadata.rotation_count, 1);

        let measurement_count = circuit
            .ops
            .iter()
            .filter(|op| matches!(op.kind, OpKind::Measurement { .. }))
            .count();
        assert_eq!(measurement_count, 3);
    }

    #[test]
    fn dependency_structure() {
        let circuit = from_tket_json_file(&fixture_path("small_circuit.json")).unwrap();

        // Each qubit has a chain of operations; verify deps exist
        assert!(!circuit.deps.is_empty());

        // All dependency edges reference valid op IDs
        let op_ids: std::collections::HashSet<OpId> = circuit.ops.iter().map(|op| op.id).collect();
        for dep in &circuit.deps {
            assert!(op_ids.contains(&dep.from), "dangling from: {}", dep.from);
            assert!(op_ids.contains(&dep.to), "dangling to: {}", dep.to);
        }
    }

    #[test]
    fn angle_conversion_halfturns_to_radians() {
        // 0.25 half-turns = π/4 radians → TGate (odd multiple of π/4)
        let angle = 0.25 * std::f64::consts::PI;
        assert!(matches!(classify_rz_angle(angle), OpKind::TGate));

        // 0.5 half-turns = π/2 radians → Clifford (even multiple of π/4)
        let angle = 0.5 * std::f64::consts::PI;
        assert!(matches!(classify_rz_angle(angle), OpKind::Clifford));

        // 0.3 half-turns = 0.3π radians → Rotation (not a multiple of π/4)
        let angle = 0.3 * std::f64::consts::PI;
        assert!(matches!(classify_rz_angle(angle), OpKind::Rotation { .. }));

        // 1.0 half-turns = π radians → Clifford (4× π/4)
        let angle = 1.0 * std::f64::consts::PI;
        assert!(matches!(classify_rz_angle(angle), OpKind::Clifford));
    }

    #[test]
    fn rx_pi_over_4_classified_as_tgate() {
        let params = Some(vec!["0.25".into()]);
        let kind = classify_op(OpType::Rx, &params).unwrap();
        assert_eq!(kind, OpKind::TGate);
    }

    #[test]
    fn ry_pi_over_4_classified_as_tgate() {
        let params = Some(vec!["0.25".into()]);
        let kind = classify_op(OpType::Ry, &params).unwrap();
        assert_eq!(kind, OpKind::TGate);
    }

    #[test]
    fn rx_pi_over_2_classified_as_clifford() {
        let params = Some(vec!["0.5".into()]);
        let kind = classify_op(OpType::Rx, &params).unwrap();
        assert_eq!(kind, OpKind::Clifford);
    }

    #[test]
    fn ry_arbitrary_classified_as_rotation() {
        let params = Some(vec!["0.3".into()]);
        let kind = classify_op(OpType::Ry, &params).unwrap();
        assert!(matches!(kind, OpKind::Rotation { .. }));
    }

    #[test]
    fn symbolic_parameter_rejected() {
        let result = parse_first_param(&Some(vec!["alpha".into()]), "Rz");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, TketJsonError::SymbolicParameter { .. }),
            "expected SymbolicParameter, got: {err}"
        );
    }

    #[test]
    fn unsupported_conditional_rejected() {
        assert!(reject_unsupported(OpType::Conditional).is_err());
    }

    #[test]
    fn unsupported_box_rejected() {
        assert!(reject_unsupported(OpType::CircBox).is_err());
        assert!(reject_unsupported(OpType::CustomGate).is_err());
    }

    #[test]
    fn unsupported_classical_rejected() {
        assert!(reject_unsupported(OpType::ClassicalTransform).is_err());
        assert!(reject_unsupported(OpType::WASM).is_err());
    }

    #[test]
    fn structural_ops_skipped() {
        assert!(is_structural(OpType::Barrier));
        assert!(is_structural(OpType::noop));
        assert!(!is_structural(OpType::Reset));
        assert!(!is_structural(OpType::H));
        assert!(!is_structural(OpType::T));
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let result = from_tket_json_str("not valid json");
        assert!(matches!(result, Err(TketJsonError::Parse(_))));
    }

    #[test]
    fn cross_adapter_consistency_with_qasm() {
        let tket_circuit = from_tket_json_file(&fixture_path("small_circuit.json")).unwrap();
        let qasm_circuit =
            crate::openqasm::from_qasm_file(&fixture_path("small_circuit.qasm")).unwrap();

        // Same gate kind counts
        assert_eq!(tket_circuit.metadata.t_count, qasm_circuit.metadata.t_count);
        assert_eq!(
            tket_circuit.metadata.clifford_count,
            qasm_circuit.metadata.clifford_count
        );
        assert_eq!(
            tket_circuit.metadata.rotation_count,
            qasm_circuit.metadata.rotation_count
        );

        // Same qubit count
        assert_eq!(tket_circuit.qubit_count, qasm_circuit.qubit_count);

        // Same number of operations
        assert_eq!(tket_circuit.ops.len(), qasm_circuit.ops.len());

        // Same number of dependencies
        assert_eq!(tket_circuit.deps.len(), qasm_circuit.deps.len());

        // Same op kinds in order
        for (tket_op, qasm_op) in tket_circuit.ops.iter().zip(qasm_circuit.ops.iter()) {
            match (&tket_op.kind, &qasm_op.kind) {
                (OpKind::Rotation { angle: a }, OpKind::Rotation { angle: b }) => {
                    assert!((a - b).abs() < 1e-10, "rotation angles differ: {a} vs {b}");
                }
                (a, b) => assert_eq!(
                    std::mem::discriminant(a),
                    std::mem::discriminant(b),
                    "op kind mismatch at id {}: {:?} vs {:?}",
                    tket_op.id,
                    a,
                    b
                ),
            }
        }
    }
}
