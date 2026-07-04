//! Legalization: prove the lowered model fits the Game Boy numeric contract
//! before kernel selection (bd-1skgm).
//!
//! Each check pairs the pinned device bound with the value observed in the
//! real model; a failed check aborts compilation with the named reason. The
//! passing checks are recorded in the build report so the legality claims
//! are auditable numbers, not vibes.

use std::error::Error;
use std::fmt;

use gbf_kernel::model_ref::{D_FF, D_MODEL, N_BLOCKS, QMAX, RESID_ONE, VOCAB};
use serde::Serialize;

use crate::lower_infer::DenseBigramProgram;
use crate::lower_quant::QuantLoweredModel;

/// One legality check: named bound vs the observed value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LegalizationCheck {
    pub name: &'static str,
    pub bound: String,
    pub observed: String,
    pub ok: bool,
}

/// Result of legalization: the checks that were run (all `ok`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LegalizationReport {
    pub checks: Vec<LegalizationCheck>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegalizeError {
    pub failed: Vec<LegalizationCheck>,
}

impl fmt::Display for LegalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "model is not legal for the device contract:")?;
        for check in &self.failed {
            write!(
                f,
                " [{} bound {} observed {}]",
                check.name, check.bound, check.observed
            )?;
        }
        Ok(())
    }
}

impl Error for LegalizeError {}

/// Run every device-contract legality check for the dense-bigram backend.
pub fn legalize(
    program: &DenseBigramProgram,
    lowered: &QuantLoweredModel,
) -> Result<LegalizationReport, LegalizeError> {
    let mut checks = Vec::new();
    let mut check = |name: &'static str, bound: String, observed: String, ok: bool| {
        checks.push(LegalizationCheck {
            name,
            bound,
            observed,
            ok,
        });
    };

    // The banked V3 backend is pinned to the bring-up dims: the driver's
    // norm walks exactly 64 lanes, activation/accumulator pages hold 128
    // entries, the head loop scans exactly 256 vocab rows.
    check(
        "dims_match_backend_contract",
        format!("d_model={D_MODEL}, d_ff={D_FF}, n_blocks={N_BLOCKS}, vocab={VOCAB}"),
        format!(
            "d_model={}, d_ff={}, n_blocks={}, vocab={}",
            program.d_model,
            program.d_ff,
            program.blocks.len(),
            program.vocab
        ),
        program.d_model == D_MODEL
            && program.d_ff == D_FF
            && program.blocks.len() == N_BLOCKS
            && program.vocab == VOCAB,
    );

    // i16 matvec accumulator: |bias + sum(w*u)| <= fan_in * 127 must fit i16.
    let max_fan_in = program
        .blocks
        .iter()
        .flat_map(|b| [b.up.cols, b.down.cols])
        .max()
        .unwrap_or(0);
    check(
        "matvec_i16_accumulator",
        format!("fan_in * {QMAX} <= 32767 (fan_in <= 258)"),
        format!(
            "max fan_in {max_fan_in} -> bound {}",
            max_fan_in as i64 * i64::from(QMAX)
        ),
        max_fan_in as i64 * i64::from(QMAX) <= 32767,
    );

    // Activation grid must be the [-8, 8] / 127 grid the GELU LUT and the
    // norm divider are generated for.
    let grids_ok = program.blocks.iter().all(|b| {
        b.norm.qmax == QMAX && b.norm.range == 8.0 && b.gelu.qmax == QMAX && b.gelu.range == 8.0
    }) && program.final_norm.qmax == QMAX
        && program.final_norm.range == 8.0;
    check(
        "activation_grid",
        format!("qmax {QMAX}, range 8.0 on every norm/gelu op"),
        if grids_ok {
            "all ops on the [-8,8]/127 grid".to_string()
        } else {
            "at least one op off-grid".to_string()
        },
        grids_ok,
    );

    // Down-epilogue u32 bound: |scale * acc| * 2 + 127 < 2^32 with the
    // observed maximum scale and the structural |acc| <= fan_in * 127.
    let max_acc = max_fan_in as u64 * QMAX as u64;
    let down_num = u64::from(lowered.max_scale_raw) * max_acc * 2 + 127;
    check(
        "down_epilogue_u32",
        "max_scale * max_acc * 2 + 127 < 2^32".to_string(),
        format!(
            "max_scale {} -> {} (< {})",
            lowered.max_scale_raw,
            down_num,
            1u64 << 32
        ),
        down_num < 1 << 32,
    );

    // Tied-head logits must fit the device i24 representation:
    // d_model * 127 * 127 < 2^23.
    let logit_bound = program.d_model as u64 * QMAX as u64 * QMAX as u64;
    check(
        "head_logits_i24",
        format!("d_model * {QMAX}^2 < {}", 1u64 << 23),
        format!("{logit_bound}"),
        logit_bound < 1 << 23,
    );

    // Q11.5 embedding quantization: max|emb| * 32 must fit i16 (the residual
    // init rows are quantized at lowering time).
    let emb_q = f64::from(lowered.max_abs_embedding) * f64::from(RESID_ONE);
    check(
        "embedding_q11_5",
        "max|emb| * 32 <= 32767".to_string(),
        format!("max|emb| {} -> {emb_q:.1}", lowered.max_abs_embedding),
        emb_q <= 32767.0,
    );

    let failed: Vec<LegalizationCheck> = checks.iter().filter(|c| !c.ok).cloned().collect();
    if failed.is_empty() {
        Ok(LegalizationReport { checks })
    } else {
        Err(LegalizeError { failed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_checkpoint_export::{
        import_checkpoint_export, write_synthetic_checkpoint_export,
    };
    use crate::lower_infer::lower_infer;
    use crate::lower_quant::lower_quant;

    #[test]
    fn legalize_passes_the_synthetic_model_and_records_observed_values() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), 13).expect("writes export");
        let imported = import_checkpoint_export(dir.path()).expect("imports");
        let program = lower_infer(&imported.core, &imported.topology).expect("lowers infer");
        let lowered = lower_quant(&imported.core, &program).expect("lowers quant");
        let report = legalize(&program, &lowered).expect("legalizes");
        assert_eq!(report.checks.len(), 6);
        assert!(report.checks.iter().all(|c| c.ok));
    }

    #[test]
    fn legalize_rejects_wrong_dims() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), 13).expect("writes export");
        let imported = import_checkpoint_export(dir.path()).expect("imports");
        let mut program = lower_infer(&imported.core, &imported.topology).expect("lowers infer");
        let lowered = lower_quant(&imported.core, &program).expect("lowers quant");
        program.blocks.pop();
        let err = legalize(&program, &lowered).expect_err("must reject");
        assert!(
            err.failed
                .iter()
                .any(|c| c.name == "dims_match_backend_contract")
        );
    }
}
