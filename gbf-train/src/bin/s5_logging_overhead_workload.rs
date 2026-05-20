use std::env;
use std::error::Error;
use std::time::Instant;

use gbf_train::logging::{PreflightEvent, PreflightStatus, ShadowCompileEvent, TrainingLogEmitter};
use serde_json::json;
use tracing_subscriber::prelude::*;

const DEFAULT_WARMUP_ITERATIONS: usize = 5;
const DEFAULT_MEASURED_ITERATIONS: usize = 50;

fn main() -> Result<(), Box<dyn Error>> {
    let args = WorkloadArgs::parse()?;
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
            "logging_compiled_out": cfg!(feature = "s5-no-log"),
        })
    );
    Ok(())
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
    emitter.preflight(preflight)?;
    emitter.shadow_compile(shadow)?;
    Ok(())
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

impl WorkloadArgs {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut args = env::args().skip(1);
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
                _ => return Err(format!("unknown argument: {arg}").into()),
            }
        }

        Ok(parsed)
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

fn print_usage() {
    eprintln!(
        "Usage: s5_logging_overhead_workload [--warmup N] [--measured N]\n\
         Emits one JSON line with median_ns for the tiny preflight + shadow_compile workload."
    );
}
