//! Opt-in local telemetry for MAGI.
//!
//! When the `MAGI_TELEMETRY=1` environment variable is set, collects and
//! prints performance/usage statistics to stderr at the end of execution.
//! Purely local — no network access, no data leaves the machine.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Local telemetry collector.
#[derive(Debug)]
pub struct Telemetry {
    enabled: bool,
    start: Instant,
    execution_count: u64,
    total_execution_time: Duration,
    operations_used: HashMap<String, u64>,
    errors_encountered: u64,
}

impl Telemetry {
    /// Create a new telemetry collector.
    /// Automatically checks `MAGI_TELEMETRY` env var.
    pub fn new() -> Self {
        let enabled = std::env::var("MAGI_TELEMETRY")
            .map(|v| v == "1")
            .unwrap_or(false);
        Self {
            enabled,
            start: Instant::now(),
            execution_count: 0,
            total_execution_time: Duration::ZERO,
            operations_used: HashMap::new(),
            errors_encountered: 0,
        }
    }

    /// Returns whether telemetry collection is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Record a single execution run with its duration.
    pub fn record_execution(&mut self, duration: Duration) {
        if !self.enabled {
            return;
        }
        self.execution_count += 1;
        self.total_execution_time += duration;
    }

    /// Record that an operation was used.
    pub fn record_operation(&mut self, name: &str) {
        if !self.enabled {
            return;
        }
        *self.operations_used.entry(name.to_string()).or_insert(0) += 1;
    }

    /// Record an error.
    pub fn record_error(&mut self) {
        if !self.enabled {
            return;
        }
        self.errors_encountered += 1;
    }

    /// Print a summary to stderr if telemetry is enabled.
    pub fn report(&self) {
        if !self.enabled {
            return;
        }

        let wall_time = self.start.elapsed();

        eprintln!("--- MAGI Telemetry ---");
        eprintln!("  Wall time:        {:.3}s", wall_time.as_secs_f64());
        eprintln!("  Executions:       {}", self.execution_count);
        if self.execution_count > 0 {
            eprintln!(
                "  Execution time:   {:.3}s",
                self.total_execution_time.as_secs_f64()
            );
            let avg = self.total_execution_time.as_secs_f64() / self.execution_count as f64;
            eprintln!("  Avg per run:      {:.3}s", avg);
        }
        eprintln!("  Errors:           {}", self.errors_encountered);

        if !self.operations_used.is_empty() {
            let mut ops: Vec<(&String, &u64)> = self.operations_used.iter().collect();
            ops.sort_by(|a, b| b.1.cmp(a.1));
            let top_n = ops.iter().take(10);
            eprintln!("  Top operations:");
            for (name, count) in top_n {
                eprintln!("    {}: {}", name, count);
            }
        }
        eprintln!("----------------------");
    }
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_disabled_by_default() {
        let t = Telemetry::new();
        assert!(!t.is_enabled());
    }

    #[test]
    fn test_telemetry_record_execution() {
        let mut t = Telemetry {
            enabled: true,
            start: Instant::now(),
            execution_count: 0,
            total_execution_time: Duration::ZERO,
            operations_used: HashMap::new(),
            errors_encountered: 0,
        };
        t.record_execution(Duration::from_millis(100));
        t.record_execution(Duration::from_millis(200));
        assert_eq!(t.execution_count, 2);
        assert_eq!(t.total_execution_time, Duration::from_millis(300));
    }

    #[test]
    fn test_telemetry_record_operation() {
        let mut t = Telemetry {
            enabled: true,
            start: Instant::now(),
            execution_count: 0,
            total_execution_time: Duration::ZERO,
            operations_used: HashMap::new(),
            errors_encountered: 0,
        };
        t.record_operation("MathAdd");
        t.record_operation("MathAdd");
        t.record_operation("StrConcat");
        assert_eq!(t.operations_used.get("MathAdd"), Some(&2));
        assert_eq!(t.operations_used.get("StrConcat"), Some(&1));
    }

    #[test]
    fn test_telemetry_record_error() {
        let mut t = Telemetry {
            enabled: true,
            start: Instant::now(),
            execution_count: 0,
            total_execution_time: Duration::ZERO,
            operations_used: HashMap::new(),
            errors_encountered: 0,
        };
        t.record_error();
        t.record_error();
        assert_eq!(t.errors_encountered, 2);
    }

    #[test]
    fn test_telemetry_noop_when_disabled() {
        let mut t = Telemetry {
            enabled: false,
            start: Instant::now(),
            execution_count: 0,
            total_execution_time: Duration::ZERO,
            operations_used: HashMap::new(),
            errors_encountered: 0,
        };
        t.record_execution(Duration::from_secs(1));
        t.record_operation("MathAdd");
        t.record_error();
        assert_eq!(t.execution_count, 0);
        assert!(t.operations_used.is_empty());
        assert_eq!(t.errors_encountered, 0);
    }

    #[test]
    fn test_telemetry_report_disabled() {
        // Should not panic when disabled
        let t = Telemetry::new();
        t.report();
    }

    #[test]
    fn test_telemetry_default() {
        let t = Telemetry::default();
        assert!(!t.is_enabled());
        assert_eq!(t.execution_count, 0);
    }
}
