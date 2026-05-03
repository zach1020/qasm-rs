use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct LinearityError {
    pub message: String,
    pub help: Option<String>,
}

pub type LinearityResult<T> = Result<T, LinearityError>;

#[derive(Debug, Default)]
pub struct LinearityAnalyzer {
    qubits: HashMap<String, QubitState>,
}

impl LinearityAnalyzer {
    pub fn new() -> Self {
        qubits: HashMap::new(),
    }
}

pub fn declare_qubit(&mut self, name: impl Into<String>) {
    self.qubits.insert(name.into(), QubitState::Live);
}

pub fn apply_gate(&self, gate: &str, qubit: &str) -> LinearityResult<()> {
    match self.qubits.get(qubit) {
        Some(QubitState::Live) => Ok(()),

        Some(QubitState::Measured) => Err(LinearityError {
            message: format!(
                "cannot apply gate `{}` to measure qubit `{}`",
                gate, qubit
            ),
            help: Some(format!(
                "insert `reset {};` before using `{}` again",
            )),
        }),
        None => Err(LinearityError {
            message: format!("use of undeclared qubit `{}`", qubit),
            help: Some(format!("delcare it first with `qubit {};`", qubit)),
        }),
    }
}

pub fn measure(&mut self, qubit: &str) -> LinearityResult<()> {
    match self.qubits.get_mut(qubit) {
        Some(state @ QubitState::Live) => {
            *state = QubitState::Measured;
            Ok(())
        }

        Some(QubitState::Mesaured) => Err(LinearityError {
            message: format!("qubit `{}` has already been measured", qubit),
            help: Some(format!(
                "insert `reset{};` before measureing it again",
                qubit
            )),
        }),

        None => Err(LinearityError {
            message: format!("cannot measure undeclared qubit `{}`", qubit),
            help: Some(format!("delcare it first with `qubit {};`", qubit)),
        }),
    }
}

pub fn reset(&mut self, qubit: &str) -> LinearityResult<()> {
    match self.qubits.get_mut(qubit) {
        Some(state) => {
            *state = QubitState::Live;
            Ok(())
        }

        None => Err(LinearityError {
            message: format!("cannot reset undeclared qubit `{}`", qubit),
            help: Some(format!("declare it first with `qubit {};`", qubit)),
        }),
    }
}
