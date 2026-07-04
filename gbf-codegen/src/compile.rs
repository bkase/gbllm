//! `gbf compile` backend: one real dataflow path from a trained checkpoint
//! export to a bootable model ROM (bd-1skgm).
//!
//! Pipeline (each stage's output is the next stage's input; nothing is
//! hand-carried):
//!
//! ```text
//! import_checkpoint_export  export dir -> ArtifactCore + ExportTopology
//! lower_infer               core + topology -> DenseBigramProgram
//! lower_quant               core + program -> IntLoweredModel (v0 contract)
//! legalize                  program + lowered -> device-bound checks
//! kernel_select             program -> KernelPlan (V3 weights-as-code etc.)
//! rom backend               plan + lowered -> banked MBC5 ROM
//!                            (gbf_kernel::asm_impl_model::build_multi_token_rom)
//! ```
//!
//! **Honest coverage note:** this path does not yet run the generic pipeline
//! stages (Stage 0 validate, storage planning, windows/overlays, arena,
//! scheduling, stage cache) — those still lack real producers for this input.
//! The build report names what is real and what is not wired
//! ([`StageCoverage`]).

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use gbf_kernel::asm_impl_model::{ModelRomError, MultiTokenRom, build_multi_token_rom};
use gbf_kernel::model_ref::IntLoweredModel;
use serde::Serialize;

use crate::import_checkpoint_export::{
    CheckpointExportFacts, CheckpointImportError, import_checkpoint_export,
};
use crate::kernel_select::{KernelFamily, KernelPlan, KernelSelection, select_kernels};
use crate::legalize::{LegalizationReport, LegalizeError, legalize};
use crate::lower_infer::{DenseBigramProgram, LowerInferError, lower_infer};
use crate::lower_quant::{LowerQuantError, lower_quant};

/// Build-report schema identifier.
pub const BUILD_REPORT_SCHEMA: &str = "gbf_compile_build_report.v1";

/// Compilation options.
#[derive(Debug, Clone, Copy)]
pub struct CompileOptions {
    /// On-device generation steps compiled into the ROM loop (1..=256).
    pub n_tokens: u16,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self { n_tokens: 256 }
    }
}

// ---------------------------------------------------------------------------
// report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactFacts {
    /// SHA-256 semantic hash of the validated artifact core.
    pub semantic_hash: String,
    pub tensor_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgramFacts {
    pub d_model: usize,
    pub d_ff: usize,
    pub n_blocks: usize,
    pub vocab: usize,
    pub ops: Vec<String>,
    pub weight_zero_permille: u32,
    pub max_scale_raw: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct RomFacts {
    pub rom_bytes: usize,
    pub bank_count: u16,
    pub driver_bytes: usize,
    pub weight_code_bytes: usize,
    pub weight_chunk_count: usize,
    pub table_bytes: usize,
    pub token_start_pc: u16,
    pub token_boundary_pc: u16,
    pub token_end_pc: u16,
    pub n_tokens: u16,
}

/// What this compile path really runs vs what remains unwired.
#[derive(Debug, Clone, Serialize)]
pub struct StageCoverage {
    /// Stages executed with real dataflow on this path.
    pub real_dataflow: Vec<&'static str>,
    /// Generic pipeline stages not on this path (no real producer yet).
    pub not_wired: Vec<&'static str>,
    pub notes: Vec<&'static str>,
}

impl StageCoverage {
    fn current() -> Self {
        Self {
            real_dataflow: vec![
                "import_checkpoint_export (sha256-verified -> ArtifactCore)",
                "lower_infer (narrow DenseBigramProgram IR; not GbInferIR — see module doc)",
                "lower_quant (v0 integer numeric contract via gbf-kernel model_ref)",
                "legalize (device-bound checks with observed values)",
                "kernel_select (V3 weights-as-code family per bake-off bd-rzq5n)",
                "rom_backend (gbf-kernel asm_impl_model banked MBC5 builder)",
            ],
            not_wired: vec![
                "validate (Stage 0 artifact-view validation)",
                "storage_plan",
                "window",
                "overlay_plan",
                "arena",
                "schedule / schedule_cost",
                "stage_cache (run_compiled_build_stage_cache_pipeline)",
            ],
            notes: vec![
                "The lowering-middle IR is a narrow model-class IR, not the Stage 3 GbInferIR; \
                 constructing GbInferIR would require fabricating Stage 0-2 products that have \
                 no real producer for a checkpoint export today.",
                "ROM section assembly reuses the gbf-kernel model-ROM builder proven byte-exact \
                 by the one-token/multi-token gates; gbf-codegen owns selection/orchestration.",
            ],
        }
    }
}

/// The program-generated build report written next to the ROM.
#[derive(Debug, Clone, Serialize)]
pub struct BuildReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub checkpoint: CheckpointExportFacts,
    pub artifact: ArtifactFacts,
    pub program: ProgramFacts,
    pub legalization: LegalizationReport,
    pub kernel_plan: Vec<KernelSelection>,
    pub rom: RomFacts,
    pub stage_coverage: StageCoverage,
}

/// Compilation result: the assembled ROM plus everything the gate runner and
/// the report writer need.
#[derive(Debug, Clone)]
pub struct CompiledModel {
    pub rom: MultiTokenRom,
    /// The canonical host evaluator for the compiled weights (the byte-exact
    /// gate compares device output against this).
    pub int_model: IntLoweredModel,
    pub report: BuildReport,
}

// ---------------------------------------------------------------------------
// errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum CompileError {
    Import(CheckpointImportError),
    LowerInfer(LowerInferError),
    LowerQuant(LowerQuantError),
    Legalize(LegalizeError),
    /// Kernel selection chose a family the ROM backend does not implement.
    UnsupportedKernelSelection {
        op: String,
        kernel: String,
    },
    Rom(ModelRomError),
    BadTokenCount {
        n_tokens: u16,
    },
    Io {
        path: PathBuf,
        reason: String,
    },
    Serialize(String),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Import(error) => write!(f, "import: {error}"),
            Self::LowerInfer(error) => write!(f, "lower_infer: {error}"),
            Self::LowerQuant(error) => write!(f, "lower_quant: {error}"),
            Self::Legalize(error) => write!(f, "legalize: {error}"),
            Self::UnsupportedKernelSelection { op, kernel } => {
                write!(f, "no backend for kernel {kernel} selected for op {op}")
            }
            Self::Rom(error) => write!(f, "rom backend: {error}"),
            Self::BadTokenCount { n_tokens } => {
                write!(f, "n_tokens {n_tokens} outside the ring capacity 1..=256")
            }
            Self::Io { path, reason } => write!(f, "io {}: {reason}", path.display()),
            Self::Serialize(reason) => write!(f, "report serialization: {reason}"),
        }
    }
}

impl Error for CompileError {}

impl From<CheckpointImportError> for CompileError {
    fn from(error: CheckpointImportError) -> Self {
        Self::Import(error)
    }
}

impl From<LowerInferError> for CompileError {
    fn from(error: LowerInferError) -> Self {
        Self::LowerInfer(error)
    }
}

impl From<LowerQuantError> for CompileError {
    fn from(error: LowerQuantError) -> Self {
        Self::LowerQuant(error)
    }
}

impl From<LegalizeError> for CompileError {
    fn from(error: LegalizeError) -> Self {
        Self::Legalize(error)
    }
}

impl From<ModelRomError> for CompileError {
    fn from(error: ModelRomError) -> Self {
        Self::Rom(error)
    }
}

// ---------------------------------------------------------------------------
// backend dispatch
// ---------------------------------------------------------------------------

/// Verify the kernel plan only names families the `gbf-kernel` model-ROM
/// builder implements. Today there is exactly one implemented family per op
/// class; a future selection of anything else must fail loudly here instead
/// of being silently replaced by the builder's fixed choice.
fn check_plan_supported(
    program: &DenseBigramProgram,
    plan: &KernelPlan,
) -> Result<(), CompileError> {
    for selection in &plan.selections {
        let supported = matches!(
            selection.kernel,
            KernelFamily::V3WeightsAsCodeBanked
                | KernelFamily::EmbeddingTableQ11_5Banked
                | KernelFamily::IntNormQuantFullVector
                | KernelFamily::ScaleEpilogueQ8_8
                | KernelFamily::GeluLut255
                | KernelFamily::ScaleEpilogueQ8_8ResidualAdd
                | KernelFamily::TiedHeadLaneMajorI8ProductLut
                | KernelFamily::ArgmaxI24LowestIndex
        );
        if !supported {
            return Err(CompileError::UnsupportedKernelSelection {
                op: selection.op.clone(),
                kernel: format!("{:?}", selection.kernel),
            });
        }
    }
    // Every op must have a selection (total plan).
    for op in program.op_names() {
        if plan.kernel_for(&op).is_none() {
            return Err(CompileError::UnsupportedKernelSelection {
                op,
                kernel: "<none>".to_string(),
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// compile
// ---------------------------------------------------------------------------

/// Compile a `f_s6_dense_checkpoint_export.v1` bundle into a bootable
/// multi-token generation ROM plus its build report.
pub fn compile_checkpoint_export(
    export_dir: &Path,
    options: &CompileOptions,
) -> Result<CompiledModel, CompileError> {
    if options.n_tokens == 0 || options.n_tokens > 256 {
        return Err(CompileError::BadTokenCount {
            n_tokens: options.n_tokens,
        });
    }

    let imported = import_checkpoint_export(export_dir)?;
    let program = lower_infer(&imported.core, &imported.topology)?;
    let lowered = lower_quant(&imported.core, &program)?;
    let legalization = legalize(&program, &lowered)?;
    let plan = select_kernels(&program);
    check_plan_supported(&program, &plan)?;
    let rom = build_multi_token_rom(&lowered.model, options.n_tokens)?;

    let report = BuildReport {
        schema: BUILD_REPORT_SCHEMA,
        bead: "bd-1skgm",
        checkpoint: imported.facts,
        artifact: ArtifactFacts {
            semantic_hash: imported.core.semantic_hash().to_string(),
            tensor_count: imported.core.tensors().len(),
        },
        program: ProgramFacts {
            d_model: program.d_model,
            d_ff: program.d_ff,
            n_blocks: program.blocks.len(),
            vocab: program.vocab,
            ops: program.op_names(),
            weight_zero_permille: lowered.weight_zero_permille,
            max_scale_raw: lowered.max_scale_raw,
        },
        legalization,
        kernel_plan: plan.selections,
        rom: RomFacts {
            rom_bytes: rom.rom.len(),
            bank_count: rom.bank_count,
            driver_bytes: rom.driver_bytes,
            weight_code_bytes: rom.weight_code_bytes,
            weight_chunk_count: rom.weight_chunk_count,
            table_bytes: rom.table_bytes,
            token_start_pc: rom.token_start_pc,
            token_boundary_pc: rom.token_boundary_pc,
            token_end_pc: rom.token_end_pc,
            n_tokens: rom.n_tokens,
        },
        stage_coverage: StageCoverage::current(),
    };

    Ok(CompiledModel {
        rom,
        int_model: lowered.model,
        report,
    })
}

/// Paths written by [`write_build_outputs`].
#[derive(Debug, Clone)]
pub struct BuildOutputs {
    pub rom_path: PathBuf,
    pub report_path: PathBuf,
}

/// Write `rom.gb` and `build_report.json` into `out_dir` (created if needed).
pub fn write_build_outputs(
    compiled: &CompiledModel,
    out_dir: &Path,
) -> Result<BuildOutputs, CompileError> {
    let io_err = |path: &Path| {
        let path = path.to_path_buf();
        move |e: std::io::Error| CompileError::Io {
            path,
            reason: e.to_string(),
        }
    };
    std::fs::create_dir_all(out_dir).map_err(io_err(out_dir))?;
    let rom_path = out_dir.join("rom.gb");
    std::fs::write(&rom_path, &compiled.rom.rom).map_err(io_err(&rom_path))?;
    let report_path = out_dir.join("build_report.json");
    let json = serde_json::to_vec_pretty(&compiled.report)
        .map_err(|e| CompileError::Serialize(e.to_string()))?;
    std::fs::write(&report_path, json).map_err(io_err(&report_path))?;
    Ok(BuildOutputs {
        rom_path,
        report_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_checkpoint_export::write_synthetic_checkpoint_export;
    use gbf_kernel::model_ref::synthetic_checkpoint;

    /// The compiled-path ROM must be byte-identical to the ROM built directly
    /// from the same weights: this proves the importer/lowering round trip is
    /// lossless end to end (any divergence anywhere in the dataflow changes
    /// weight code, tables, or scale data bytes).
    #[test]
    fn compiled_rom_is_byte_identical_to_direct_builder_output() {
        let seed = 23;
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), seed).expect("writes export");
        let compiled = compile_checkpoint_export(dir.path(), &CompileOptions { n_tokens: 256 })
            .expect("compiles");

        let direct_model =
            IntLoweredModel::lower(&synthetic_checkpoint(seed)).expect("direct lowering");
        let direct_rom = build_multi_token_rom(&direct_model, 256).expect("direct build");
        assert_eq!(compiled.rom.rom.len(), direct_rom.rom.len());
        assert!(
            compiled.rom.rom == direct_rom.rom,
            "compiled-path ROM diverges from the direct builder ROM"
        );
        assert_eq!(compiled.report.rom.n_tokens, 256);
        assert!(compiled.report.legalization.checks.iter().all(|c| c.ok));
        assert!(!compiled.report.kernel_plan.is_empty());
    }

    #[test]
    fn compile_rejects_bad_token_counts() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), 23).expect("writes export");
        assert!(matches!(
            compile_checkpoint_export(dir.path(), &CompileOptions { n_tokens: 0 }),
            Err(CompileError::BadTokenCount { n_tokens: 0 })
        ));
        assert!(matches!(
            compile_checkpoint_export(dir.path(), &CompileOptions { n_tokens: 257 }),
            Err(CompileError::BadTokenCount { n_tokens: 257 })
        ));
    }

    #[test]
    fn write_build_outputs_writes_rom_and_report() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), 29).expect("writes export");
        let compiled = compile_checkpoint_export(dir.path(), &CompileOptions { n_tokens: 8 })
            .expect("compiles");
        let out = tempfile::tempdir().expect("out tempdir");
        let outputs = write_build_outputs(&compiled, out.path()).expect("writes outputs");
        let rom_bytes = std::fs::read(&outputs.rom_path).expect("rom readable");
        assert_eq!(rom_bytes, compiled.rom.rom);
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&outputs.report_path).expect("report readable"))
                .expect("report parses");
        assert_eq!(report["schema"], BUILD_REPORT_SCHEMA);
        assert_eq!(report["rom"]["n_tokens"], 8);
        assert!(
            report["artifact"]["semantic_hash"]
                .as_str()
                .expect("hash string")
                .starts_with("sha256:")
        );
    }

    /// Compiling the committed real checkpoint export must succeed and the
    /// compiled ROM must match the direct builder on the real weights, too.
    #[test]
    fn compiles_the_committed_s6_checkpoint_export() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let export_dir = repo_root.join("experiments/S6/checkpoint-export");
        if !export_dir.join("manifest.json").exists() {
            panic!(
                "committed checkpoint export missing at {}",
                export_dir.display()
            );
        }
        let compiled = compile_checkpoint_export(&export_dir, &CompileOptions::default())
            .expect("real checkpoint compiles");
        assert_eq!(compiled.report.checkpoint.tensors_verified_sha256, 17);
        assert_eq!(compiled.report.program.n_blocks, 4);
        assert!(compiled.report.rom.rom_bytes > 0);
        // Spot-check the host evaluator wired through the pipeline: forward
        // passes must be deterministic and produce i24-range logits.
        let trace = compiled.int_model.forward(0x65);
        assert!(
            trace
                .logits
                .iter()
                .all(|&l| (-(1 << 23)..(1 << 23)).contains(&l))
        );
    }
}
