//! OpenQASM 3 adapter — converts `.qasm` files into Profiler IR.
//!
//! Uses `oq3_semantics` (IBM/Qiskit official Rust parser). Covers QASM 2.0
//! as a subset.

use std::path::Path;

use oq3_semantics::{
    asg::{
        self, ArithOp, BinaryOp, Expr, GateModifier, GateOperand, IndexOperator, Literal, Stmt,
        TExpr, UnaryOp,
    },
    symbols::{SymbolId, SymbolTable, SymbolType as _},
    syntax_to_semantics::parse_source_string,
    types::{ArrayDims, Type},
};
use pirx_ir::{
    circuit::{
        CircuitMetadata, Dependency, OpId, OpKind, Operation, ProfilerCircuit, QubitId,
        classify_rz_angle,
    },
    validate::ValidatedCircuit,
};
use smallvec::SmallVec;

use crate::error::OpenQasmError;

/// Parse an OpenQASM file and return a validated profiler circuit.
///
/// Uses the file's parent directory as include search path so that
/// `include "stdgates.inc"` and similar directives resolve correctly.
#[must_use = "parsing a circuit without using the result is always a bug"]
pub fn from_qasm_file(path: &Path) -> Result<ValidatedCircuit, OpenQasmError> {
    let source = std::fs::read_to_string(path)?;
    let search_paths: Vec<&Path> = path.parent().into_iter().collect();
    from_qasm_str_inner(&source, Some(search_paths.as_slice()))
}

/// Parse an OpenQASM source string and return a validated profiler circuit.
#[must_use = "parsing a circuit without using the result is always a bug"]
pub fn from_qasm_str(source: &str) -> Result<ValidatedCircuit, OpenQasmError> {
    from_qasm_str_inner(source, None::<&[&Path]>)
}

fn from_qasm_str_inner<P: AsRef<Path>>(
    source: &str,
    search_paths: Option<&[P]>,
) -> Result<ValidatedCircuit, OpenQasmError> {
    let result = parse_source_string(source, None, search_paths);
    if result.any_errors() {
        let mut msgs = Vec::new();
        collect_error_messages(&result, &mut msgs);
        return Err(OpenQasmError::Parse(msgs.join("; ")));
    }

    let context = result.take_context();
    let (program, _errors, symbol_table) = context.as_tuple();

    let mut builder = CircuitBuilder::new(&symbol_table);
    builder.walk_program(&program)?;
    builder.finish()
}

fn collect_error_messages<T: oq3_source_file::SourceTrait>(
    result: &oq3_semantics::syntax_to_semantics::ParseResult<T>,
    msgs: &mut Vec<String>,
) {
    for err in result.program().stmts() {
        if let Stmt::NullStmt = err {
            msgs.push("syntax error".into());
        }
    }
    if msgs.is_empty() && result.any_errors() {
        msgs.push("OpenQASM parse/semantic error".into());
    }
}

// ── Qubit registry ──────────────────────────────────────────────────────────

struct QubitRegistry {
    qubit_count: u32,
    symbols: std::collections::HashMap<SymbolId, (QubitId, u32)>,
}

impl QubitRegistry {
    fn new() -> Self {
        Self {
            qubit_count: 0,
            symbols: std::collections::HashMap::new(),
        }
    }

    fn register(&mut self, sym_id: SymbolId, count: u32) {
        let start = self.qubit_count;
        self.symbols.insert(sym_id, (start, count));
        self.qubit_count = self.qubit_count.saturating_add(count);
    }

    fn resolve(&self, sym_id: &SymbolId, index: Option<u32>) -> Result<QubitId, OpenQasmError> {
        let &(start, count) = self
            .symbols
            .get(sym_id)
            .ok_or_else(|| OpenQasmError::Parse("unresolved qubit identifier".into()))?;
        match index {
            Some(i) if i < count => Ok(start + i),
            Some(i) => Err(OpenQasmError::Parse(format!(
                "qubit index {i} out of bounds (register size {count})"
            ))),
            None if count == 1 => Ok(start),
            None => Err(OpenQasmError::Parse(
                "bare reference to multi-qubit register — use indexing".into(),
            )),
        }
    }
}

// ── Circuit builder ─────────────────────────────────────────────────────────

struct CircuitBuilder<'a> {
    symbol_table: &'a SymbolTable,
    qubits: QubitRegistry,
    ops: Vec<Operation>,
    deps: Vec<Dependency>,
    last_on_qubit: Vec<Option<OpId>>,
    next_id: OpId,
    t_count: u64,
    clifford_count: u64,
    rotation_count: u64,
    depth_at_qubit: Vec<u64>,
}

impl<'a> CircuitBuilder<'a> {
    fn new(symbol_table: &'a SymbolTable) -> Self {
        Self {
            symbol_table,
            qubits: QubitRegistry::new(),
            ops: Vec::new(),
            deps: Vec::new(),
            last_on_qubit: Vec::new(),
            next_id: 0,
            t_count: 0,
            clifford_count: 0,
            rotation_count: 0,
            depth_at_qubit: Vec::new(),
        }
    }

    fn walk_program(&mut self, program: &asg::Program) -> Result<(), OpenQasmError> {
        for stmt in program.stmts() {
            self.walk_stmt(stmt)?;
        }
        Ok(())
    }

    /// Gates inside conditional blocks are included unconditionally —
    /// pirx profiles the maximum execution path.
    fn walk_stmt(&mut self, stmt: &Stmt) -> Result<(), OpenQasmError> {
        match stmt {
            Stmt::DeclareQuantum(dq) => self.declare_qubit(dq),
            Stmt::GateCall(gc) => self.process_gate_call(gc),
            Stmt::Assignment(assign) => self.process_assignment(assign),
            Stmt::DeclareClassical(_)
            | Stmt::GateDefinition(_)
            | Stmt::Include(_)
            | Stmt::Pragma(_)
            | Stmt::Barrier(_)
            | Stmt::NullStmt
            | Stmt::End => Ok(()),
            Stmt::Reset(reset) => {
                let qubits = self.resolve_measure_operand(reset.gate_operand())?;
                for &q in &qubits {
                    self.emit_operation(OpKind::Measurement { hook: None }, &[q]);
                }
                Ok(())
            }
            Stmt::If(if_stmt) => self.walk_block(if_stmt.then_branch()),
            Stmt::While(w) => self.walk_block(w.loop_body()),
            Stmt::Block(b) => self.walk_block(b),
            // Unrecognized QASM 3 statements are ignored — pirx extracts gates only.
            _ => Ok(()),
        }
    }

    fn walk_block(&mut self, block: &asg::Block) -> Result<(), OpenQasmError> {
        for stmt in block.statements() {
            self.walk_stmt(stmt)?;
        }
        Ok(())
    }

    fn declare_qubit(&mut self, dq: &asg::DeclareQuantum) -> Result<(), OpenQasmError> {
        let sym_id = dq
            .name()
            .as_ref()
            .map_err(|_| OpenQasmError::Parse("unresolved qubit declaration".into()))?
            .clone();
        // oq3_semantics SymbolTable only exposes Index (no .get()) — IDs are valid by parser construction
        #[allow(clippy::indexing_slicing)]
        let typ = &self.symbol_table[&sym_id];
        let count: u32 = match typ.symbol_type() {
            Type::Qubit => 1,
            Type::QubitArray(ArrayDims::D1(n)) => u32::try_from(*n)
                .map_err(|_| OpenQasmError::Parse("qubit register too large".into()))?,
            _ => {
                return Err(OpenQasmError::Parse(format!(
                    "unexpected type for qubit declaration: {typ:?}"
                )));
            }
        };
        self.qubits.register(sym_id, count);
        self.last_on_qubit
            .resize(self.qubits.qubit_count as usize, None);
        self.depth_at_qubit
            .resize(self.qubits.qubit_count as usize, 0);
        Ok(())
    }

    fn process_gate_call(&mut self, gc: &asg::GateCall) -> Result<(), OpenQasmError> {
        let gate_name = self.resolve_gate_name(gc)?;
        let has_inv = gc
            .modifiers()
            .iter()
            .any(|m| matches!(m, GateModifier::Inv));
        let params = self.evaluate_params(gc, gate_name)?;
        let kind = classify_gate(gate_name, &params, has_inv);
        let qubits = self.resolve_gate_qubits(gc)?;

        self.emit_operation(kind, &qubits);
        Ok(())
    }

    fn process_assignment(&mut self, assign: &asg::Assignment) -> Result<(), OpenQasmError> {
        if let Expr::MeasureExpression(measure) = assign.rvalue().expression() {
            let qubits = self.resolve_measure_operand(measure.operand())?;
            for &q in &qubits {
                self.emit_operation(OpKind::Measurement { hook: None }, &[q]);
            }
        }
        Ok(())
    }

    fn emit_operation(&mut self, kind: OpKind, qubits: &[QubitId]) {
        let id = self.next_id;

        let qubits_sv: SmallVec<[QubitId; 2]> = SmallVec::from_slice(qubits);
        let op = Operation {
            id,
            kind,
            qubits: qubits_sv,
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

    fn resolve_gate_name(&self, gc: &asg::GateCall) -> Result<&str, OpenQasmError> {
        let sym_id = gc
            .name()
            .as_ref()
            .map_err(|_| OpenQasmError::Parse("unresolved gate name".into()))?;
        #[allow(clippy::indexing_slicing)]
        Ok(self.symbol_table[sym_id].name())
    }

    fn evaluate_params(
        &self,
        gc: &asg::GateCall,
        gate_name: &str,
    ) -> Result<Vec<f64>, OpenQasmError> {
        let Some(params) = gc.params() else {
            return Ok(Vec::new());
        };
        params
            .iter()
            .map(|texpr| eval_expr(texpr.expression(), gate_name, self.symbol_table))
            .collect()
    }

    fn resolve_gate_qubits(&self, gc: &asg::GateCall) -> Result<SmallVec<[QubitId; 2]>, OpenQasmError> {
        let mut qubits = SmallVec::new();
        for texpr in gc.qubits() {
            match texpr.expression() {
                Expr::GateOperand(GateOperand::Identifier(sym_result)) => {
                    let sym_id = sym_result
                        .as_ref()
                        .map_err(|_| OpenQasmError::Parse("unresolved qubit".into()))?;
                    qubits.push(self.qubits.resolve(sym_id, None)?);
                }
                Expr::GateOperand(GateOperand::IndexedIdentifier(idx_id)) => {
                    let sym_id = idx_id
                        .identifier()
                        .as_ref()
                        .map_err(|_| OpenQasmError::Parse("unresolved qubit".into()))?;
                    let index = extract_index(idx_id.indexes())?;
                    qubits.push(self.qubits.resolve(sym_id, Some(index))?);
                }
                Expr::GateOperand(GateOperand::HardwareQubit(hw)) => {
                    let id_str = hw.identifier();
                    let id_str = id_str.strip_prefix('$').unwrap_or(id_str);
                    let q: QubitId = id_str.parse().map_err(|_| {
                        OpenQasmError::Parse(format!("invalid hardware qubit: {}", hw.identifier()))
                    })?;
                    qubits.push(q);
                }
                _ => {
                    return Err(OpenQasmError::Parse(
                        "unexpected expression in gate qubit operand".into(),
                    ));
                }
            }
        }
        Ok(qubits)
    }

    fn resolve_measure_operand(&self, operand: &TExpr) -> Result<Vec<QubitId>, OpenQasmError> {
        match operand.expression() {
            Expr::GateOperand(GateOperand::Identifier(sym_result)) => {
                let sym_id = sym_result
                    .as_ref()
                    .map_err(|_| OpenQasmError::Parse("unresolved qubit in measure".into()))?;
                Ok(vec![self.qubits.resolve(sym_id, None)?])
            }
            Expr::GateOperand(GateOperand::IndexedIdentifier(idx_id)) => {
                let sym_id = idx_id
                    .identifier()
                    .as_ref()
                    .map_err(|_| OpenQasmError::Parse("unresolved qubit in measure".into()))?;
                let index = extract_index(idx_id.indexes())?;
                Ok(vec![self.qubits.resolve(sym_id, Some(index))?])
            }
            _ => Err(OpenQasmError::Parse(
                "unexpected expression in measure operand".into(),
            )),
        }
    }

    fn finish(self) -> Result<ValidatedCircuit, OpenQasmError> {
        let max_depth = self.depth_at_qubit.iter().copied().max().unwrap_or(0);
        let circuit = ProfilerCircuit {
            ops: self.ops,
            deps: self.deps,
            qubit_count: self.qubits.qubit_count,
            qubit_positions: None,
            hooks: vec![],
            metadata: CircuitMetadata {
                name: String::new(),
                source_framework: "openqasm".into(),
                t_count: self.t_count,
                clifford_count: self.clifford_count,
                rotation_count: self.rotation_count,
                depth: max_depth,
            },
        };
        pirx_ir::validate::validate(circuit).map_err(OpenQasmError::from)
    }
}

// ── Gate classification ─────────────────────────────────────────────────────

fn classify_gate(name: &str, params: &[f64], has_inv: bool) -> OpKind {
    match name {
        "t" | "tdg" => OpKind::TGate,
        "rz" | "rx" | "ry" | "p" | "u1" | "phase" => {
            let Some(&angle) = params.first() else {
                return OpKind::Clifford;
            };
            let effective_angle = if has_inv { -angle } else { angle };
            classify_rz_angle(effective_angle)
        }
        "measure" => OpKind::Measurement { hook: None },
        _ => OpKind::Clifford,
    }
}

// ── Expression evaluation ───────────────────────────────────────────────────

fn eval_expr(
    expr: &Expr,
    gate_name: &str,
    symbol_table: &SymbolTable,
) -> Result<f64, OpenQasmError> {
    match expr {
        Expr::Literal(Literal::Float(f)) => f
            .value()
            .parse::<f64>()
            .map_err(|_| OpenQasmError::Parse(format!("invalid float literal: {}", f.value()))),
        Expr::Literal(Literal::Int(i)) => {
            let val = *i.value() as f64;
            if *i.sign() { Ok(-val) } else { Ok(val) }
        }
        Expr::Identifier(sym_result) => {
            let sym_id = sym_result
                .as_ref()
                .map_err(|_| OpenQasmError::SymbolicParameter {
                    gate: gate_name.into(),
                    name: "<unresolved>".into(),
                })?;
            #[allow(clippy::indexing_slicing)]
            let symbol = &symbol_table[sym_id];
            match symbol.name() {
                "pi" | "π" => Ok(std::f64::consts::PI),
                "tau" | "τ" => Ok(std::f64::consts::TAU),
                "euler" | "ℇ" => Ok(std::f64::consts::E),
                name => Err(OpenQasmError::SymbolicParameter {
                    gate: gate_name.into(),
                    name: name.into(),
                }),
            }
        }
        Expr::BinaryExpr(bin) => {
            let left = eval_expr(bin.left().expression(), gate_name, symbol_table)?;
            let right = eval_expr(bin.right().expression(), gate_name, symbol_table)?;
            match bin.op() {
                BinaryOp::ArithOp(ArithOp::Add) => Ok(left + right),
                BinaryOp::ArithOp(ArithOp::Sub) => Ok(left - right),
                BinaryOp::ArithOp(ArithOp::Mul) => Ok(left * right),
                BinaryOp::ArithOp(ArithOp::Div) => Ok(left / right),
                _ => Err(OpenQasmError::Parse(format!(
                    "unsupported binary operator in gate '{gate_name}' parameter"
                ))),
            }
        }
        Expr::UnaryExpr(un) => {
            let val = eval_expr(un.operand().expression(), gate_name, symbol_table)?;
            match un.op() {
                UnaryOp::Minus => Ok(-val),
                _ => Err(OpenQasmError::Parse(format!(
                    "unsupported unary operator in gate '{gate_name}' parameter"
                ))),
            }
        }
        Expr::Cast(cast) => eval_expr(cast.operand().expression(), gate_name, symbol_table),
        _ => Err(OpenQasmError::SymbolicParameter {
            gate: gate_name.into(),
            name: format!("{expr:?}"),
        }),
    }
}

// ── Index extraction ────────────────────────────────────────────────────────

fn extract_index(indexes: &[IndexOperator]) -> Result<u32, OpenQasmError> {
    let index_op = indexes
        .first()
        .ok_or_else(|| OpenQasmError::Parse("missing index on qubit reference".into()))?;
    match index_op {
        IndexOperator::ExpressionList(elist) => {
            let texpr = elist
                .expressions
                .first()
                .ok_or_else(|| OpenQasmError::Parse("empty index expression".into()))?;
            match texpr.expression() {
                Expr::Literal(Literal::Int(i)) => u32::try_from(*i.value()).map_err(|_| {
                    OpenQasmError::Parse(format!("qubit index too large: {}", i.value()))
                }),
                _ => Err(OpenQasmError::Parse("non-integer qubit index".into())),
            }
        }
        IndexOperator::SetExpression(_) => Err(OpenQasmError::Parse(
            "set expression not supported as qubit index".into(),
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn parse(source: &str) -> ValidatedCircuit {
        from_qasm_str(source).unwrap()
    }

    fn qasm3_header() -> &'static str {
        "OPENQASM 3.0;\ninclude \"stdgates.inc\";\n"
    }

    fn make_source(body: &str) -> String {
        format!("{}{body}", qasm3_header())
    }

    // ── Gate classification ─────────────────────────────────────────────

    #[test]
    fn t_gate_classified_as_tgate() {
        let src = make_source("qubit q;\nt q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops.len(), 1);
        assert_eq!(circuit.ops[0].kind, OpKind::TGate);
        assert_eq!(circuit.metadata.t_count, 1);
    }

    #[test]
    fn tdg_classified_as_tgate() {
        let src = make_source("qubit q;\ntdg q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops.len(), 1);
        assert_eq!(circuit.ops[0].kind, OpKind::TGate);
        assert_eq!(circuit.metadata.t_count, 1);
    }

    #[test]
    fn rz_pi_over_4_classified_as_tgate() {
        let src = make_source("qubit q;\nrz(pi/4) q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops.len(), 1);
        assert_eq!(circuit.ops[0].kind, OpKind::TGate);
        assert_eq!(circuit.metadata.t_count, 1);
    }

    #[test]
    fn rz_3pi_over_4_classified_as_tgate() {
        let src = make_source("qubit q;\nrz(3*pi/4) q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops[0].kind, OpKind::TGate);
    }

    #[test]
    fn rz_arbitrary_classified_as_rotation() {
        let src = make_source("qubit q;\nrz(0.3) q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops.len(), 1);
        assert!(matches!(circuit.ops[0].kind, OpKind::Rotation { .. }));
        assert_eq!(circuit.metadata.rotation_count, 1);
    }

    #[test]
    fn rz_pi_over_2_classified_as_clifford() {
        let src = make_source("qubit q;\nrz(pi/2) q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops[0].kind, OpKind::Clifford);
    }

    #[test]
    fn rx_pi_over_4_classified_as_tgate() {
        let src = make_source("qubit q;\nrx(pi/4) q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops[0].kind, OpKind::TGate);
        assert_eq!(circuit.metadata.t_count, 1);
    }

    #[test]
    fn ry_pi_over_4_classified_as_tgate() {
        let src = make_source("qubit q;\nry(pi/4) q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops[0].kind, OpKind::TGate);
        assert_eq!(circuit.metadata.t_count, 1);
    }

    #[test]
    fn rx_pi_over_2_classified_as_clifford() {
        let src = make_source("qubit q;\nrx(pi/2) q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops[0].kind, OpKind::Clifford);
    }

    #[test]
    fn ry_arbitrary_classified_as_rotation() {
        let src = make_source("qubit q;\nry(0.3) q;\n");
        let circuit = parse(&src);
        assert!(matches!(circuit.ops[0].kind, OpKind::Rotation { .. }));
        assert_eq!(circuit.metadata.rotation_count, 1);
    }

    #[test]
    fn h_classified_as_clifford() {
        let src = make_source("qubit q;\nh q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops[0].kind, OpKind::Clifford);
        assert_eq!(circuit.metadata.clifford_count, 1);
    }

    #[test]
    fn cx_classified_as_clifford() {
        let src = make_source("qubit[2] q;\ncx q[0], q[1];\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops[0].kind, OpKind::Clifford);
    }

    #[test]
    fn s_classified_as_clifford() {
        let src = make_source("qubit q;\ns q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops[0].kind, OpKind::Clifford);
    }

    #[test]
    fn measure_classified_as_measurement() {
        let src = make_source("qubit q;\nbit c;\nc = measure q;\n");
        let circuit = parse(&src);
        assert!(matches!(
            circuit.ops[0].kind,
            OpKind::Measurement { hook: None }
        ));
    }

    // ── Dependency inference ────────────────────────────────────────────

    #[test]
    fn same_qubit_creates_dependency() {
        let src = make_source("qubit q;\nh q;\nt q;\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops.len(), 2);
        assert_eq!(circuit.deps.len(), 1);
        assert_eq!(circuit.deps[0].from, 0);
        assert_eq!(circuit.deps[0].to, 1);
    }

    #[test]
    fn disjoint_qubits_no_dependency() {
        let src = make_source("qubit[2] q;\nh q[0];\nt q[1];\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops.len(), 2);
        assert!(circuit.deps.is_empty());
    }

    #[test]
    fn cnot_then_h_creates_dependency_on_shared_qubit() {
        let src = make_source("qubit[2] q;\ncx q[0], q[1];\nh q[0];\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops.len(), 2);
        assert_eq!(circuit.deps.len(), 1);
        assert_eq!(circuit.deps[0].from, 0);
        assert_eq!(circuit.deps[0].to, 1);
    }

    #[test]
    fn cnot_creates_deps_on_both_qubits() {
        let src = make_source("qubit[2] q;\nh q[0];\nh q[1];\ncx q[0], q[1];\n");
        let circuit = parse(&src);
        assert_eq!(circuit.ops.len(), 3);
        assert_eq!(circuit.deps.len(), 2);
        let dep_pairs: Vec<(u64, u64)> = circuit.deps.iter().map(|d| (d.from, d.to)).collect();
        assert!(dep_pairs.contains(&(0, 2)));
        assert!(dep_pairs.contains(&(1, 2)));
    }

    // ── Symbolic rejection ──────────────────────────────────────────────

    #[test]
    fn symbolic_parameter_rejected() {
        let src = make_source("input float theta;\nqubit q;\nrz(theta) q;\n");
        let result = from_qasm_str(&src);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, OpenQasmError::SymbolicParameter { .. }),
            "expected SymbolicParameter, got: {err}"
        );
    }

    // ── Validation ──────────────────────────────────────────────────────

    #[test]
    fn valid_circuit_passes_validation() {
        let src = make_source(
            "qubit[2] q;\nbit[2] c;\nh q[0];\ncx q[0], q[1];\nt q[0];\nrz(0.3) q[1];\nc[0] = measure q[0];\nc[1] = measure q[1];\n",
        );
        let circuit = parse(&src);
        assert_eq!(circuit.qubit_count, 2);
        assert_eq!(circuit.metadata.t_count, 1);
        assert_eq!(circuit.metadata.rotation_count, 1);
    }

    // ── QASM 2.0 compatibility ──────────────────────────────────────────

    #[test]
    fn qasm2_round_trip() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");
        let tmp = tempfile::tempdir().unwrap();

        // Copy qelib1.inc so the parser's include resolution finds it.
        std::fs::copy(fixtures.join("qelib1.inc"), tmp.path().join("qelib1.inc")).unwrap();

        let src = "\
OPENQASM 2.0;
include \"qelib1.inc\";
qreg q[2];
creg c[2];
h q[0];
cx q[0], q[1];
t q[0];
tdg q[1];
measure q[0] -> c[0];
measure q[1] -> c[1];
";
        let qasm_path = tmp.path().join("test.qasm");
        std::fs::write(&qasm_path, src).unwrap();
        // oq3_semantics 0.7 does not support QASM 2.0 qreg/creg/measure->
        let result = from_qasm_file(&qasm_path);
        assert!(
            result.is_err(),
            "expected QASM 2.0 syntax to be rejected by oq3_semantics"
        );
    }

    // ── Fixture file ────────────────────────────────────────────────────

    #[test]
    fn fixture_file_round_trip() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("small_circuit.qasm");
        assert!(fixture.exists(), "fixture not found: {}", fixture.display());
        let circuit = from_qasm_file(&fixture).unwrap();
        assert!(!circuit.ops.is_empty());
        assert!(circuit.qubit_count > 0);
    }
}
