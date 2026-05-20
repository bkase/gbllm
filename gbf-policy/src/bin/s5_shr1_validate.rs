use std::env;
use std::fs;
use std::path::Path;

use gbf_policy::{
    ShadowCompileSampleExpectation, ShadowCompileSampleReal, validate_shr1_shadow_sample,
};
use serde_json::json;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        return Err(format!(
            "usage: {} <broken-negative-control.sample.json> <assertion-output.json>",
            args.first()
                .map(String::as_str)
                .unwrap_or("s5_shr1_validate")
        )
        .into());
    }

    let fixture_path = Path::new(&args[1]);
    let output_path = Path::new(&args[2]);
    let sample: ShadowCompileSampleReal = serde_json::from_str(&fs::read_to_string(fixture_path)?)?;
    validate_shr1_shadow_sample(
        &sample,
        ShadowCompileSampleExpectation::BrokenNegativeControl,
    )?;

    let assertion = json!({
        "schema": "s5_negative_control_assertion.v1",
        "fixture": fixture_path.display().to_string(),
        "fixture_role": "canonical_rehearsal_validator_json",
        "validated_by": "gbf-policy::shadow::validate_shr1_shadow_sample",
        "variant": sample.variant,
        "seed": sample.seed,
        "shadow_compile_ok": sample.shadow_compile_ok,
        "diagnostic": sample.shadow_compile_skipped,
        "failure_stage": sample.failure_stage,
    });

    fs::write(
        output_path,
        serde_json::to_string(&assertion).map(|payload| payload + "\n")?,
    )?;
    Ok(())
}
