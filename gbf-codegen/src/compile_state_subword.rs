//! Production compiler path for the JOYP-driven recurrent subword cartridge.
//!
//! The benchmark crate may execute and measure this ROM, but it does not own
//! any import, lowering, or assembly step in this path.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use gbf_data::bpe::BpeModel;
use gbf_foundation::sha256;
use gbf_kernel::asm_impl_shell::{
    SUBWORD_FONT_BYTES, SUBWORD_NEWLINE_BYTE, SubwordShellRom,
    build_state_subword_shell_rom_with_seed,
};
use gbf_kernel::asm_impl_state::{
    PagedHeadStorage, S_RNG_ADDR, S_SAMPLED_ADDR, S_SAMPLED_HI_ADDR, WeightLowering,
};
use gbf_kernel::decode::{SamplerConfig, SamplerConfigError};
use gbf_kernel::state_model_ref::{IntStateLoweredModel, LogitPaging, StateTopology};
use serde::Serialize;

use crate::import_state_checkpoint::{StateCheckpointImportError, import_state_checkpoint};

pub const INTERACTIVE_SUBWORD_PROFILE: &str = "interactive-subword-dmg";
pub const INTERACTIVE_SUBWORD_BUILD_REPORT_SCHEMA: &str = "gbf_interactive_subword_build_report.v1";
pub const INTERACTIVE_SUBWORD_COMPILE_REQUEST_SCHEMA: &str =
    "gbf_interactive_subword_compile_request.v1";

/// The deployed cartridge's pinned defaults.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct InteractiveSubwordCompileOptions {
    pub n_tokens: u8,
    pub top_k: u8,
    pub temperature: f64,
    pub rng_seed: u16,
}

impl Default for InteractiveSubwordCompileOptions {
    fn default() -> Self {
        Self {
            n_tokens: 24,
            top_k: 4,
            temperature: 0.6,
            rng_seed: 0x5EED,
        }
    }
}

/// Narrow, truthful IR for the model class this backend actually accepts.
///
/// This is intentionally not `DenseBigramProgram` and not a fabricated
/// `GbInferIR`: it represents recurrent state, wide u16 token ids, byte-BPE,
/// on-device prefill/generation, and the JOYP shell directly.
#[derive(Debug, Clone)]
pub struct StatefulSubwordProgram {
    pub topology: StateTopology,
    pub id_bytes: Vec<Vec<u8>>,
    pub merges: Vec<(u16, u16)>,
    pub max_token_bytes: usize,
}

impl StatefulSubwordProgram {
    fn from_tokenizer(
        topology: StateTopology,
        tokenizer: &BpeModel,
    ) -> Result<Self, InteractiveSubwordCompileError> {
        if topology.is_moe() || topology.logit_paging != LogitPaging::Paged {
            return Err(InteractiveSubwordCompileError::UnsupportedProgram {
                reason: "interactive subword profile requires a dense paged-vocabulary recurrent checkpoint"
                    .to_string(),
            });
        }
        if tokenizer.vocab_size() != topology.vocab {
            return Err(InteractiveSubwordCompileError::UnsupportedProgram {
                reason: format!(
                    "tokenizer vocab {} does not match checkpoint vocab {}",
                    tokenizer.vocab_size(),
                    topology.vocab
                ),
            });
        }
        let id_bytes = (0..topology.vocab)
            .map(|id| {
                tokenizer
                    .id_bytes(id as u16)
                    .expect("id is inside validated tokenizer vocab")
                    .to_vec()
            })
            .collect();
        Ok(Self {
            topology,
            id_bytes,
            merges: tokenizer.merges().to_vec(),
            max_token_bytes: tokenizer.max_token_len(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractiveSubwordCompileRequest {
    pub schema: &'static str,
    pub profile: &'static str,
    pub checkpoint_export: String,
    pub tokenizer: String,
    pub options: InteractiveSubwordCompileOptions,
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractiveSubwordArtifactFacts {
    pub checkpoint_schema: String,
    pub trainer_git_sha: Option<String>,
    pub checkpoint_manifest_sha256: String,
    pub tensors_verified_sha256: usize,
    pub tokenizer_format: String,
    pub tokenizer_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatefulSubwordProgramFacts {
    pub d_model: usize,
    pub d_ff: usize,
    pub n_blocks: usize,
    pub state_slots: usize,
    pub n_experts: usize,
    pub vocab: usize,
    pub token_id_width_bits: u8,
    pub tokenizer_merges: usize,
    pub max_token_bytes: usize,
    pub operations: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractiveSubwordSamplerFacts {
    pub top_k: u8,
    pub requested_temperature: f64,
    pub effective_temperature: f64,
    pub scale_q16: u16,
    pub rng: &'static str,
    pub rng_seed: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractiveSubwordRomFacts {
    pub sha256: String,
    pub rom_bytes: usize,
    pub bank_count: u16,
    pub rom_size: String,
    pub cartridge: &'static str,
    pub cartridge_ram_bytes: usize,
    pub driver_bytes: usize,
    pub ui_bank_bytes: usize,
    pub tokenizer_bank_bytes: usize,
    pub table_bytes: usize,
    pub weight_lowering: &'static str,
    pub paged_head_storage: &'static str,
    pub prompt_capacity_bytes: u8,
    pub generation_tokens: u8,
    pub idle_pc: u16,
    pub tokenize_done_pc: u16,
    pub warm_boundary_pc: u16,
    pub forward_pass_pc: u16,
    pub token_boundary_pc: u16,
    pub generation_done_pc: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractiveSubwordStageCoverage {
    pub real_dataflow: Vec<&'static str>,
    pub not_wired: Vec<&'static str>,
    pub notes: Vec<&'static str>,
}

impl InteractiveSubwordStageCoverage {
    fn current() -> Self {
        Self {
            real_dataflow: vec![
                "import_state_checkpoint (manifest topology + per-tensor sha256)",
                "StatefulSubwordProgram construction (dense recurrent + byte-BPE)",
                "IntStateLoweredModel integer lowering",
                "WRAM and SramFull wide-head storage planning",
                "V3 weights-as-code stateful subword backend",
                "gbf-asm MBC5+RAM ROM assembly",
            ],
            not_wired: vec![
                "generic Stage 0 ArtifactView validation",
                "generic GbInferIR lowering",
                "generic window / overlay / arena scheduling",
                "generic stage cache",
            ],
            notes: vec![
                "The model-class IR is deliberately narrow; no generic compiler products are fabricated.",
                "The benchmark crate consumes the compiler output only for emulator and latency gates.",
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractiveSubwordBuildReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub profile: &'static str,
    pub compiler_crate: &'static str,
    pub compiler_version: &'static str,
    pub compiler_git_revision: Option<&'static str>,
    pub artifact: InteractiveSubwordArtifactFacts,
    pub program: StatefulSubwordProgramFacts,
    pub sampler: InteractiveSubwordSamplerFacts,
    pub rom: InteractiveSubwordRomFacts,
    pub stage_coverage: InteractiveSubwordStageCoverage,
}

#[derive(Debug, Clone)]
pub struct CompiledInteractiveSubword {
    pub rom: SubwordShellRom,
    pub lowered_model: IntStateLoweredModel,
    pub program: StatefulSubwordProgram,
    pub request: InteractiveSubwordCompileRequest,
    pub report: InteractiveSubwordBuildReport,
}

#[derive(Debug)]
pub enum InteractiveSubwordCompileError {
    Import(StateCheckpointImportError),
    Io { path: PathBuf, reason: String },
    Tokenizer(String),
    UnsupportedProgram { reason: String },
    Lowering(String),
    Sampler(SamplerConfigError),
    Rom(gbf_kernel::asm_impl_model::ModelRomError),
    Serialize(String),
}

impl fmt::Display for InteractiveSubwordCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Import(error) => write!(f, "state checkpoint import: {error}"),
            Self::Io { path, reason } => write!(f, "io {}: {reason}", path.display()),
            Self::Tokenizer(reason) => write!(f, "tokenizer: {reason}"),
            Self::UnsupportedProgram { reason } => write!(f, "program: {reason}"),
            Self::Lowering(reason) => write!(f, "integer lowering: {reason}"),
            Self::Sampler(error) => write!(f, "sampler: {error}"),
            Self::Rom(error) => write!(f, "ROM backend: {error}"),
            Self::Serialize(reason) => write!(f, "output serialization: {reason}"),
        }
    }
}

impl Error for InteractiveSubwordCompileError {}

impl From<StateCheckpointImportError> for InteractiveSubwordCompileError {
    fn from(error: StateCheckpointImportError) -> Self {
        Self::Import(error)
    }
}

impl From<SamplerConfigError> for InteractiveSubwordCompileError {
    fn from(error: SamplerConfigError) -> Self {
        Self::Sampler(error)
    }
}

impl From<gbf_kernel::asm_impl_model::ModelRomError> for InteractiveSubwordCompileError {
    fn from(error: gbf_kernel::asm_impl_model::ModelRomError) -> Self {
        Self::Rom(error)
    }
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> InteractiveSubwordCompileError {
    let path = path.to_path_buf();
    move |error| InteractiveSubwordCompileError::Io {
        path,
        reason: error.to_string(),
    }
}

/// Build the byte-indexed 8x8 font embedded in the production cartridge.
#[must_use]
pub fn subword_font_tiles() -> Vec<u8> {
    const NEWLINE_GLYPH: [u8; 16] = [
        0x00, 0x00, 0x02, 0x02, 0x02, 0x02, 0x12, 0x12, 0x3E, 0x3E, 0x10, 0x10, 0x00, 0x00, 0x00,
        0x00,
    ];
    let font = gbf_runtime::text::font_bytes();
    let mut out = Vec::with_capacity(SUBWORD_FONT_BYTES);
    for byte in 0..128u8 {
        if byte == SUBWORD_NEWLINE_BYTE {
            out.extend_from_slice(&NEWLINE_GLYPH);
        } else if (0x20..0x7F).contains(&byte) {
            let ascii = byte as usize;
            out.extend_from_slice(&font[ascii * 16..ascii * 16 + 16]);
        } else {
            out.extend_from_slice(&[0u8; 16]);
        }
    }
    out
}

fn storage_name(storage: PagedHeadStorage) -> &'static str {
    match storage {
        PagedHeadStorage::WramStreamed => "WramStreamed",
        PagedHeadStorage::SramFull => "SramFull",
    }
}

fn lowering_name(lowering: WeightLowering) -> &'static str {
    match lowering {
        WeightLowering::V3 => "V3",
        WeightLowering::V2Dispatch => "V2Dispatch",
    }
}

/// Compile a verified recurrent export and tokenizer into the interactive ROM.
pub fn compile_interactive_subword(
    checkpoint_export: &Path,
    tokenizer_path: &Path,
    options: &InteractiveSubwordCompileOptions,
) -> Result<CompiledInteractiveSubword, InteractiveSubwordCompileError> {
    let bundle = import_state_checkpoint(checkpoint_export)?;
    let tokenizer_bytes = std::fs::read(tokenizer_path).map_err(io_error(tokenizer_path))?;
    let tokenizer_json: serde_json::Value = serde_json::from_slice(&tokenizer_bytes)
        .map_err(|error| InteractiveSubwordCompileError::Tokenizer(error.to_string()))?;
    let tokenizer_format = tokenizer_json["format"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if tokenizer_format != "gbllm_bpe.v2" {
        return Err(InteractiveSubwordCompileError::Tokenizer(format!(
            "unexpected format {tokenizer_format:?}"
        )));
    }
    let tokenizer_text = std::str::from_utf8(&tokenizer_bytes)
        .map_err(|error| InteractiveSubwordCompileError::Tokenizer(error.to_string()))?;
    let tokenizer = BpeModel::from_json(tokenizer_text)
        .map_err(|error| InteractiveSubwordCompileError::Tokenizer(error.to_string()))?;
    let program = StatefulSubwordProgram::from_tokenizer(bundle.topology, &tokenizer)?;
    let lowered_model = IntStateLoweredModel::lower(&bundle.checkpoint)
        .map_err(|error| InteractiveSubwordCompileError::Lowering(error.to_string()))?;
    let sampler = SamplerConfig::from_temperature(
        options.top_k,
        lowered_model.logit_dequant_step(),
        options.temperature,
    )?;
    let rom = build_state_subword_shell_rom_with_seed(
        &lowered_model,
        &sampler,
        options.n_tokens,
        &subword_font_tiles(),
        &program.id_bytes,
        &program.merges,
        options.rng_seed,
    )?;

    let request = InteractiveSubwordCompileRequest {
        schema: INTERACTIVE_SUBWORD_COMPILE_REQUEST_SCHEMA,
        profile: INTERACTIVE_SUBWORD_PROFILE,
        checkpoint_export: checkpoint_export.display().to_string(),
        tokenizer: tokenizer_path.display().to_string(),
        options: *options,
    };
    let topology = program.topology;
    let report = InteractiveSubwordBuildReport {
        schema: INTERACTIVE_SUBWORD_BUILD_REPORT_SCHEMA,
        bead: "bd-3mi",
        profile: INTERACTIVE_SUBWORD_PROFILE,
        compiler_crate: env!("CARGO_PKG_NAME"),
        compiler_version: env!("CARGO_PKG_VERSION"),
        compiler_git_revision: option_env!("GBF_COMPILER_GIT_REVISION"),
        artifact: InteractiveSubwordArtifactFacts {
            checkpoint_schema: bundle.manifest_schema,
            trainer_git_sha: (!bundle.manifest_git_sha.is_empty())
                .then_some(bundle.manifest_git_sha),
            checkpoint_manifest_sha256: bundle.manifest_sha256,
            tensors_verified_sha256: bundle.tensors_verified,
            tokenizer_format,
            tokenizer_sha256: sha256(&tokenizer_bytes).to_hex(),
        },
        program: StatefulSubwordProgramFacts {
            d_model: topology.d_model,
            d_ff: topology.d_ff,
            n_blocks: topology.n_blocks,
            state_slots: topology.state_slots,
            n_experts: topology.n_experts,
            vocab: topology.vocab,
            token_id_width_bits: 16,
            tokenizer_merges: program.merges.len(),
            max_token_bytes: program.max_token_bytes,
            operations: vec![
                "JOYP prompt entry",
                "byte-BPE tokenization",
                "recurrent prefill",
                "integer stateful forward",
                "top-k temperature sampling",
                "u16 token feedback",
                "literal-byte transcript render",
            ],
        },
        sampler: InteractiveSubwordSamplerFacts {
            top_k: sampler.k(),
            requested_temperature: options.temperature,
            effective_temperature: sampler
                .effective_temperature(lowered_model.logit_dequant_step()),
            scale_q16: sampler.scale_q16(),
            rng: "XorShift16(7,9,8)",
            rng_seed: rom.rng_seed,
        },
        rom: InteractiveSubwordRomFacts {
            sha256: sha256(&rom.rom).to_hex(),
            rom_bytes: rom.rom.len(),
            bank_count: rom.bank_count,
            rom_size: format!("{:?}", rom.rom_size),
            cartridge: "MBC5+RAM",
            cartridge_ram_bytes: 8 * 1024,
            driver_bytes: rom.driver_bytes,
            ui_bank_bytes: rom.ui_bank_bytes,
            tokenizer_bank_bytes: rom.tokenizer_bank_bytes,
            table_bytes: rom.table_bytes,
            weight_lowering: lowering_name(rom.weight_lowering),
            paged_head_storage: storage_name(rom.paged_head_storage),
            prompt_capacity_bytes: gbf_kernel::asm_impl_shell::SUBWORD_SHELL_PROMPT_CAP,
            generation_tokens: rom.n_gen_tokens,
            idle_pc: rom.idle_pc,
            tokenize_done_pc: rom.tokenize_done_pc,
            warm_boundary_pc: rom.warm_boundary_pc,
            forward_pass_pc: rom.forward_pass_pc,
            token_boundary_pc: rom.token_boundary_pc,
            generation_done_pc: rom.gen_done_pc,
        },
        stage_coverage: InteractiveSubwordStageCoverage::current(),
    };

    Ok(CompiledInteractiveSubword {
        rom,
        lowered_model,
        program,
        request,
        report,
    })
}

/// RGBDS-compatible symbols used by the debugger acceptance harness.
#[must_use]
pub fn interactive_subword_symbols(rom: &SubwordShellRom) -> String {
    format!(
        "00:{:04x} subword_shell_idle\n\
         00:{:04x} subword_tokenize_done\n\
         00:{:04x} subword_warm_boundary\n\
         00:{:04x} subword_forward_pass\n\
         00:{:04x} subword_token_boundary\n\
         00:{:04x} subword_generation_done\n\
         00:{:04x} subword_prompt_bytes\n\
         00:{:04x} subword_prompt_byte_len\n\
         00:{:04x} subword_prompt_token_ids\n\
         00:{:04x} subword_prompt_token_len\n\
         00:{S_RNG_ADDR:04x} subword_rng\n\
         00:{S_SAMPLED_ADDR:04x} subword_sampled_lo\n\
         00:{S_SAMPLED_HI_ADDR:04x} subword_sampled_hi\n",
        rom.idle_pc,
        rom.tokenize_done_pc,
        rom.warm_boundary_pc,
        rom.forward_pass_pc,
        rom.token_boundary_pc,
        rom.gen_done_pc,
        rom.prompt_bytes_addr,
        rom.prompt_byte_len_addr,
        rom.prompt_ids_addr,
        rom.prompt_token_len_addr,
    )
}

#[derive(Debug, Clone)]
pub struct InteractiveSubwordBuildOutputs {
    pub rom_path: PathBuf,
    pub symbols_path: PathBuf,
    pub report_path: PathBuf,
    pub request_path: PathBuf,
}

/// Write the compiler-owned output packet into `out_dir`.
pub fn write_interactive_subword_outputs(
    compiled: &CompiledInteractiveSubword,
    out_dir: &Path,
) -> Result<InteractiveSubwordBuildOutputs, InteractiveSubwordCompileError> {
    std::fs::create_dir_all(out_dir).map_err(io_error(out_dir))?;
    let rom_path = out_dir.join("rom.gb");
    let symbols_path = out_dir.join("rom.sym");
    let report_path = out_dir.join("build_report.json");
    let request_path = out_dir.join("compile_request.json");
    std::fs::write(&rom_path, &compiled.rom.rom).map_err(io_error(&rom_path))?;
    std::fs::write(&symbols_path, interactive_subword_symbols(&compiled.rom))
        .map_err(io_error(&symbols_path))?;
    let report = serde_json::to_vec_pretty(&compiled.report)
        .map_err(|error| InteractiveSubwordCompileError::Serialize(error.to_string()))?;
    std::fs::write(&report_path, report).map_err(io_error(&report_path))?;
    let request = serde_json::to_vec_pretty(&compiled.request)
        .map_err(|error| InteractiveSubwordCompileError::Serialize(error.to_string()))?;
    std::fs::write(&request_path, request).map_err(io_error(&request_path))?;
    Ok(InteractiveSubwordBuildOutputs {
        rom_path,
        symbols_path,
        report_path,
        request_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployed_options_are_pinned() {
        assert_eq!(
            InteractiveSubwordCompileOptions::default(),
            InteractiveSubwordCompileOptions {
                n_tokens: 24,
                top_k: 4,
                temperature: 0.6,
                rng_seed: 0x5EED,
            }
        );
    }

    #[test]
    fn compiler_owns_the_exact_subword_font_asset() {
        let font = subword_font_tiles();
        assert_eq!(font.len(), SUBWORD_FONT_BYTES);
        let newline = usize::from(SUBWORD_NEWLINE_BYTE) * 16;
        assert!(font[newline..newline + 16].iter().any(|&byte| byte != 0));
    }

    #[test]
    fn compile_request_json_shape_is_pinned() {
        let request = InteractiveSubwordCompileRequest {
            schema: INTERACTIVE_SUBWORD_COMPILE_REQUEST_SCHEMA,
            profile: INTERACTIVE_SUBWORD_PROFILE,
            checkpoint_export: "/artifact/ckpt".to_string(),
            tokenizer: "/artifact/tokenizer.json".to_string(),
            options: InteractiveSubwordCompileOptions::default(),
        };
        assert_eq!(
            serde_json::to_value(request).expect("request serializes"),
            serde_json::json!({
                "schema": "gbf_interactive_subword_compile_request.v1",
                "profile": "interactive-subword-dmg",
                "checkpoint_export": "/artifact/ckpt",
                "tokenizer": "/artifact/tokenizer.json",
                "options": {
                    "n_tokens": 24,
                    "top_k": 4,
                    "temperature": 0.6,
                    "rng_seed": 24301
                }
            })
        );
    }
}
