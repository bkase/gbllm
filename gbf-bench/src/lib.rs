//! Workload-driven calibration, constrained autotuning, and Pareto reporting.

pub mod autotune;
pub mod calibration;
pub mod compile_gate;
pub mod d192;
pub mod d192_real;
pub mod demo;
pub mod kernel_bakeoff;
pub mod latency;
pub mod moe_parity;
pub mod multi_token;
pub mod one_token;
pub mod reports;
pub mod sampling;
pub mod shell;
pub mod stateful;
pub mod workload;
