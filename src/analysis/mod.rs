#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_on_live_qubit_is_ok() {
        let mut analyzer = LinearityAnalyzer::new();
        analyzer.declare_qubit("q");

        assert!(analyzer.apply_gate("h", "q").is_ok());
    }

    #[test]
    fn use_after_measure_is_rejected() {
        let mut analyzer = LinearityAnalyzer::new();
        analyzer.declare_qubit("q");

        analyzer.measure("q").unwrap();

        let err = analyzer.apply_gate("x", "q").unwrap_err();

        assert!(err.message.contains("measured qubit"));
        assert!(err.help.unwrap().contains("reset q;"));
    }

    #[test]
    fn reset_restores_qubit_to_live_state() {
        let mut analyzer = LinearityAnalyzer::new();
        analyzer.declare_qubit("q");

        analyzer.measure("q").unwrap();
        analyzer.reset("q").unwrap();

        assert!(analyzer.apply_gate("x", "q").is_ok());
    }

    #[test]
    fn undeclared_qubit_is_rejected() {
        let analyzer = LinearityAnalyzer::new();

        let err = analyzer.apply_gate("h", "q").unwrap_err();

        assert!(err.message.contains("undeclared qubit"));
    }
}
