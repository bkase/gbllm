use std::env;
use std::error::Error;
use std::hint::black_box;
use std::time::Instant;

use gbf_policy::model_profile::ModelSizeProfile;
use gbf_train::logging::{
    LoggingOverheadGate, LoggingOverheadMeasurement, PreflightEvent, PreflightStatus,
    ShadowCompileEvent, TrainingLogEmitter,
};
use gbf_train::preflight::compute_preflight_profile_expert_bytes;
use serde_json::json;
use tracing_subscriber::prelude::*;

const DEFAULT_WARMUP_ITERATIONS: usize = 5;
const DEFAULT_MEASURED_ITERATIONS: usize = 50;
const REPRESENTATIVE_PAYLOAD_ITERATIONS: u64 = 512;

fn main() -> Result<(), Box<dyn Error>> {
    match Command::parse()? {
        Command::Workload(args) => run_measurement(args),
        Command::Gate(args) => run_gate(args),
    }
}

fn run_measurement(args: WorkloadArgs) -> Result<(), Box<dyn Error>> {
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(std::io::sink),
    );

    let samples = tracing::subscriber::with_default(subscriber, || run_workload(&args))?;
    let median_ns = median(samples);
    println!(
        "{}",
        json!({
            "schema": "s5_logging_overhead_workload.v1",
            "workload_id": "tiny_preflight_shadow_compile",
            "warmup_iterations": args.warmup_iterations,
            "measured_iterations": args.measured_iterations,
            "median_ns": median_ns,
            "representative_payload": "moe_tiny_preflight_budget_hash_loop",
            "representative_payload_iterations": REPRESENTATIVE_PAYLOAD_ITERATIONS,
            "logging_compiled_out": cfg!(feature = "s5-no-log"),
        })
    );
    Ok(())
}

fn run_gate(args: GateArgs) -> Result<(), Box<dyn Error>> {
    let measurement = LoggingOverheadMeasurement::new(args.baseline_ns, args.instrumented_ns)?;
    let gate = LoggingOverheadGate::constitution_one_percent();
    let report = gate.evaluate(measurement);
    let gate_result = gate.require_pass(measurement);
    let refusal_reason = gate_result
        .as_ref()
        .err()
        .map(std::string::ToString::to_string);

    println!(
        "{}",
        json!({
            "schema": "s5_logging_overhead_gate.v1",
            "baseline_ns": report.measurement().baseline_step_ns(),
            "instrumented_ns": report.measurement().instrumented_step_ns(),
            "overhead": report.overhead_fraction(),
            "threshold": report.max_overhead_fraction(),
            "pass": gate_result.is_ok(),
            "logging_compiled_out": cfg!(feature = "s5-no-log"),
            "refusal_reason": refusal_reason,
        })
    );

    if gate_result.is_ok() {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn run_workload(args: &WorkloadArgs) -> Result<Vec<u128>, Box<dyn Error>> {
    let emitter = TrainingLogEmitter::new();
    let preflight = PreflightEvent {
        check_name: "expert_slot_budget".to_owned(),
        status: PreflightStatus::Pass,
        detail: "tiny D14 preflight fits".to_owned(),
        numeric_value: 15_090.0,
        threshold: 16_384.0,
    };
    let shadow = ShadowCompileEvent {
        step: 30,
        checkpoint_id: "ckpt-d14-tiny".to_owned(),
        compile_profile: "tiny-ci".to_owned(),
        fit_status: "fits".to_owned(),
        quality_summary: "frontier stable".to_owned(),
        frontier_size: 3,
        duration_ms: 42,
    };

    for _ in 0..args.warmup_iterations {
        invoke_pair(&emitter, &preflight, &shadow)?;
    }

    let mut samples = Vec::with_capacity(args.measured_iterations);
    for _ in 0..args.measured_iterations {
        let started = Instant::now();
        invoke_pair(&emitter, &preflight, &shadow)?;
        samples.push(started.elapsed().as_nanos());
    }
    Ok(samples)
}

fn invoke_pair(
    emitter: &TrainingLogEmitter,
    preflight: &PreflightEvent,
    shadow: &ShadowCompileEvent,
) -> Result<(), Box<dyn Error>> {
    let payload_checksum = representative_tiny_preflight_payload()?;
    emitter.preflight(preflight)?;
    emitter.shadow_compile(shadow)?;
    black_box(payload_checksum);
    Ok(())
}

fn representative_tiny_preflight_payload() -> Result<u64, Box<dyn Error>> {
    let profile = ModelSizeProfile::moe_tiny(4)?;
    let expert_bytes = compute_preflight_profile_expert_bytes(profile)?.as_u64();
    let mut checksum = 0xD14_5EED_u64;

    for iteration in 0..REPRESENTATIVE_PAYLOAD_ITERATIONS {
        let common_bank_demand = 12_000_u64 + (iteration % 17);
        let bank0_demand = 2_048_u64 + (iteration % 11);
        let hot_arena_demand = 3_712_u64 + (iteration % 7);
        let payload = json!({
            "profile": "MoeTiny4",
            "expert_bytes": expert_bytes,
            "common_bank_demand": common_bank_demand,
            "bank0_demand": bank0_demand,
            "hot_arena_demand": hot_arena_demand,
            "iteration": iteration,
        });
        let encoded = serde_json::to_vec(&payload)?;
        checksum = checksum.rotate_left(7)
            ^ encoded.iter().fold(iteration, |acc, byte| {
                acc.wrapping_mul(16777619) ^ u64::from(*byte)
            });
    }

    Ok(checksum)
}

fn median(mut samples: Vec<u128>) -> u128 {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[derive(Debug, Clone, Copy)]
struct WorkloadArgs {
    warmup_iterations: usize,
    measured_iterations: usize,
}

enum Command {
    Workload(WorkloadArgs),
    Gate(GateArgs),
}

#[derive(Debug, Clone, Copy)]
struct GateArgs {
    baseline_ns: u64,
    instrumented_ns: u64,
}

impl Command {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let args: Vec<String> = env::args().skip(1).collect();
        if args.iter().any(|arg| arg == "--gate") {
            return Ok(Self::Gate(GateArgs::parse_from(args)?));
        }
        Ok(Self::Workload(WorkloadArgs::parse_from(args)?))
    }
}

impl WorkloadArgs {
    fn parse_from(args: Vec<String>) -> Result<Self, Box<dyn Error>> {
        let mut args = args.into_iter();
        let mut parsed = Self {
            warmup_iterations: DEFAULT_WARMUP_ITERATIONS,
            measured_iterations: DEFAULT_MEASURED_ITERATIONS,
        };

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--warmup" => {
                    parsed.warmup_iterations = parse_positive_usize("--warmup", args.next())?;
                }
                "--measured" => {
                    parsed.measured_iterations = parse_positive_usize("--measured", args.next())?;
                }
                "-h" | "--help" => {
                    print_usage();
                    std::process::exit(0);
                }
                "--gate" | "--baseline-ns" | "--instrumented-ns" => {
                    return Err(format!("{arg} is only valid in gate mode").into());
                }
                _ => return Err(format!("unknown argument: {arg}").into()),
            }
        }

        Ok(parsed)
    }
}

impl GateArgs {
    fn parse_from(args: Vec<String>) -> Result<Self, Box<dyn Error>> {
        let mut args = args.into_iter();
        let mut saw_gate = false;
        let mut baseline_ns = None;
        let mut instrumented_ns = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--gate" => {
                    saw_gate = true;
                }
                "--baseline-ns" => {
                    baseline_ns = Some(parse_positive_u64("--baseline-ns", args.next())?);
                }
                "--instrumented-ns" => {
                    instrumented_ns = Some(parse_positive_u64("--instrumented-ns", args.next())?);
                }
                "-h" | "--help" => {
                    print_usage();
                    std::process::exit(0);
                }
                "--warmup" | "--measured" => {
                    return Err(format!("{arg} is only valid in workload mode").into());
                }
                _ => return Err(format!("unknown argument: {arg}").into()),
            }
        }

        if !saw_gate {
            return Err("--gate is required for gate mode".into());
        }

        Ok(Self {
            baseline_ns: baseline_ns.ok_or("--baseline-ns is required for gate mode")?,
            instrumented_ns: instrumented_ns
                .ok_or("--instrumented-ns is required for gate mode")?,
        })
    }
}

fn parse_positive_usize(
    flag: &'static str,
    value: Option<String>,
) -> Result<usize, Box<dyn Error>> {
    let value = value.ok_or_else(|| format!("{flag} requires a value"))?;
    let parsed = value.parse::<usize>()?;
    if parsed == 0 {
        return Err(format!("{flag} must be positive").into());
    }
    Ok(parsed)
}

fn parse_positive_u64(flag: &'static str, value: Option<String>) -> Result<u64, Box<dyn Error>> {
    let value = value.ok_or_else(|| format!("{flag} requires a value"))?;
    let parsed = value.parse::<u64>()?;
    if parsed == 0 {
        return Err(format!("{flag} must be positive").into());
    }
    Ok(parsed)
}

fn print_usage() {
    eprintln!(
        "Usage: s5_logging_overhead_workload [--warmup N] [--measured N]\n\
         Usage: s5_logging_overhead_workload --gate --baseline-ns N --instrumented-ns N\n\
         Emits one JSON line with median_ns for the tiny preflight + shadow_compile workload,\n\
         or evaluates measured medians with LoggingOverheadGate::constitution_one_percent()."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_args_require_positive_measurements() {
        assert!(GateArgs::parse_from(vec!["--gate".to_owned()]).is_err());
        assert!(
            GateArgs::parse_from(vec![
                "--gate".to_owned(),
                "--baseline-ns".to_owned(),
                "0".to_owned(),
                "--instrumented-ns".to_owned(),
                "1".to_owned(),
            ])
            .is_err()
        );

        let args = GateArgs::parse_from(vec![
            "--gate".to_owned(),
            "--baseline-ns".to_owned(),
            "10000".to_owned(),
            "--instrumented-ns".to_owned(),
            "10050".to_owned(),
        ])
        .expect("valid gate args parse");
        assert_eq!(args.baseline_ns, 10_000);
        assert_eq!(args.instrumented_ns, 10_050);
    }

    #[test]
    fn workload_args_reject_gate_flags() {
        assert!(
            WorkloadArgs::parse_from(vec![
                "--gate".to_owned(),
                "--baseline-ns".to_owned(),
                "10000".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn representative_payload_is_deterministic_and_nonzero() {
        let first = representative_tiny_preflight_payload().unwrap();
        let second = representative_tiny_preflight_payload().unwrap();

        assert_ne!(first, 0);
        assert_eq!(first, second);
    }
}
