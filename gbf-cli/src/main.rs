use std::process::ExitCode;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CompileProfile {
    /// Stateless dense-bigram compiler path (`f_s6_dense_checkpoint_export.v1`).
    DenseBigram,
    /// JOYP-driven recurrent byte-BPE cartridge for DMG/MBC5+8KiB RAM.
    InteractiveSubwordDmg,
}

#[derive(Debug, Parser)]
#[command(name = "gbf", about = "GBLLM command-line tools")]
struct GbfCli {
    /// Structured log format for commands that emit CLI events.
    #[arg(long, default_value = "pretty", value_parser = ["pretty", "json"])]
    log_format: String,
    /// Structured log level for commands that emit CLI events.
    #[arg(long, default_value = "info", value_parser = ["off", "error", "warn", "info", "debug", "trace"])]
    log_level: String,
    /// Additional CLI event sink.
    #[arg(long)]
    log_file: Option<PathBuf>,
    /// NDJSON event capture sink for test and CI assertions.
    #[arg(long)]
    capture_events: Option<PathBuf>,
    #[command(subcommand)]
    command: GbfCommand,
}

#[derive(Debug, Subcommand)]
enum GbfCommand {
    /// Compile a trained checkpoint export into a bootable model ROM through
    /// the gbf-codegen pipeline.
    #[cfg(feature = "compile")]
    Compile {
        /// Compiler path to execute.
        #[arg(long, value_enum, default_value_t = CompileProfile::DenseBigram)]
        profile: CompileProfile,
        /// Checkpoint export directory (`manifest.json` plus tensor blobs).
        #[arg(long)]
        checkpoint_export: PathBuf,
        /// Byte-BPE JSON; required by `interactive-subword-dmg`.
        #[arg(long)]
        tokenizer: Option<PathBuf>,
        /// Output packet directory. Interactive builds write `rom.gb`,
        /// `rom.sym`, `build_report.json`, and `compile_request.json`.
        #[arg(long)]
        out: PathBuf,
        /// On-device generation steps (defaults: dense=256, interactive=24).
        #[arg(long)]
        tokens: Option<u16>,
        /// Interactive integer sampler top-k.
        #[arg(long)]
        top_k: Option<u8>,
        /// Interactive sampler temperature.
        #[arg(long)]
        temperature: Option<f64>,
        /// Interactive XorShift16 seed (decimal or `0x` hexadecimal).
        #[arg(long, value_parser = parse_u16)]
        rng_seed: Option<u16>,
    },
    /// S1 First Pulse experiment workflows.
    #[cfg(any(
        feature = "phase-a",
        feature = "ablation",
        feature = "s2-full",
        feature = "s2-ablation",
        feature = "falsify"
    ))]
    S1 {
        #[command(subcommand)]
        command: gbf_experiments::s1::cli::S1Command,
    },
    /// S2 QAT-survives experiment workflows.
    #[cfg(any(
        feature = "phase-a",
        feature = "ablation",
        feature = "s2-full",
        feature = "s2-ablation",
        feature = "falsify"
    ))]
    S2 {
        #[command(subcommand)]
        command: gbf_experiments::s2::cli::S2Command,
    },
    /// S3 TinyStories success experiment workflows.
    #[cfg(feature = "s3")]
    S3 {
        #[command(subcommand)]
        command: gbf_experiments::s3::cli::S3Command,
    },
    /// S4 Gutenberg promotion experiment workflows.
    #[cfg(feature = "s4")]
    S4 {
        #[command(subcommand)]
        command: gbf_experiments::s4::cli::S4Command,
    },
    /// S7 MoE matched-bytes experiment workflows.
    #[cfg(feature = "s7")]
    S7 {
        #[command(subcommand)]
        command: gbf_experiments::s7::cli::S7Command,
    },
}

fn main() -> ExitCode {
    match GbfCli::try_parse() {
        Ok(cli) => {
            #[cfg(any(
                feature = "phase-a",
                feature = "ablation",
                feature = "s2-full",
                feature = "s2-ablation",
                feature = "falsify"
            ))]
            let s2_logging = s2_logging(&cli);
            #[cfg(feature = "s3")]
            let s3_logging = s3_logging(&cli);
            #[cfg(feature = "s4")]
            let s4_logging = s4_logging(&cli);
            match cli.command {
                #[cfg(feature = "compile")]
                GbfCommand::Compile {
                    profile,
                    checkpoint_export,
                    tokenizer,
                    out,
                    tokens,
                    top_k,
                    temperature,
                    rng_seed,
                } => exit_code(run_compile(CompileInvocation {
                    profile,
                    checkpoint_export: &checkpoint_export,
                    tokenizer: tokenizer.as_deref(),
                    out: &out,
                    tokens,
                    top_k,
                    temperature,
                    rng_seed,
                })),
                #[cfg(any(
                    feature = "phase-a",
                    feature = "ablation",
                    feature = "s2-full",
                    feature = "s2-ablation",
                    feature = "falsify"
                ))]
                GbfCommand::S1 { command } => exit_code(gbf_experiments::s1::cli::run(
                    gbf_experiments::s1::cli::S1Cli { command },
                )),
                #[cfg(any(
                    feature = "phase-a",
                    feature = "ablation",
                    feature = "s2-full",
                    feature = "s2-ablation",
                    feature = "falsify"
                ))]
                GbfCommand::S2 { command } => exit_code(gbf_experiments::s2::cli::run(
                    gbf_experiments::s2::cli::S2Cli {
                        command,
                        logging: s2_logging,
                    },
                )),
                #[cfg(feature = "s3")]
                GbfCommand::S3 { command } => exit_code(gbf_experiments::s3::cli::run(
                    gbf_experiments::s3::cli::S3Cli {
                        command,
                        logging: s3_logging,
                    },
                )),
                #[cfg(feature = "s4")]
                GbfCommand::S4 { command } => exit_code(gbf_experiments::s4::cli::run(
                    gbf_experiments::s4::cli::S4Cli {
                        command,
                        logging: s4_logging,
                    },
                )),
                #[cfg(feature = "s7")]
                GbfCommand::S7 { command } => exit_code(gbf_experiments::s7::cli::run(
                    gbf_experiments::s7::cli::S7Cli { command },
                )),
            }
        }
        Err(error) => {
            let _ = error.print();
            exit_code_from_clap(error.kind())
        }
    }
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
))]
fn s2_logging(cli: &GbfCli) -> gbf_experiments::s2::cli::S2CliLogging {
    use gbf_experiments::s2::cli::{S2CliLogFormat, S2CliLogLevel, S2CliLogging};
    let format = match cli.log_format.as_str() {
        "json" => S2CliLogFormat::Json,
        _ => S2CliLogFormat::Pretty,
    };
    let level = match cli.log_level.as_str() {
        "off" => S2CliLogLevel::Off,
        "error" => S2CliLogLevel::Error,
        "warn" => S2CliLogLevel::Warn,
        "debug" => S2CliLogLevel::Debug,
        "trace" => S2CliLogLevel::Trace,
        _ => S2CliLogLevel::Info,
    };
    S2CliLogging {
        format,
        level,
        log_file: cli.log_file.clone(),
        capture_events: cli.capture_events.clone(),
    }
}

#[cfg(feature = "s3")]
fn s3_logging(cli: &GbfCli) -> gbf_experiments::s3::cli::S3CliLogging {
    use gbf_experiments::s3::cli::{S3CliLogFormat, S3CliLogLevel, S3CliLogging};
    let format = match cli.log_format.as_str() {
        "json" => S3CliLogFormat::Json,
        _ => S3CliLogFormat::Pretty,
    };
    let level = match cli.log_level.as_str() {
        "off" => S3CliLogLevel::Off,
        "error" => S3CliLogLevel::Error,
        "warn" => S3CliLogLevel::Warn,
        "debug" => S3CliLogLevel::Debug,
        "trace" => S3CliLogLevel::Trace,
        _ => S3CliLogLevel::Info,
    };
    S3CliLogging {
        format,
        level,
        log_file: cli.log_file.clone(),
        capture_events: cli.capture_events.clone(),
    }
}

#[cfg(feature = "s4")]
fn s4_logging(cli: &GbfCli) -> gbf_experiments::s4::cli::S4CliLogging {
    use gbf_experiments::s4::cli::{S4CliLogFormat, S4CliLogLevel, S4CliLogging};
    let format = match cli.log_format.as_str() {
        "json" => S4CliLogFormat::Json,
        _ => S4CliLogFormat::Pretty,
    };
    let level = match cli.log_level.as_str() {
        "off" => S4CliLogLevel::Off,
        "error" => S4CliLogLevel::Error,
        "warn" => S4CliLogLevel::Warn,
        "debug" => S4CliLogLevel::Debug,
        "trace" => S4CliLogLevel::Trace,
        _ => S4CliLogLevel::Info,
    };
    S4CliLogging {
        format,
        level,
        log_file: cli.log_file.clone(),
        capture_events: cli.capture_events.clone(),
    }
}

#[cfg(feature = "compile")]
#[derive(Debug)]
enum RunCompileError {
    Dense(gbf_codegen::compile::CompileError),
    Interactive(gbf_codegen::compile_state_subword::InteractiveSubwordCompileError),
    Usage(String),
}

#[cfg(feature = "compile")]
impl std::fmt::Display for RunCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dense(error) => error.fmt(f),
            Self::Interactive(error) => error.fmt(f),
            Self::Usage(reason) => f.write_str(reason),
        }
    }
}

#[cfg(feature = "compile")]
impl std::error::Error for RunCompileError {}

#[cfg(feature = "compile")]
struct CompileInvocation<'a> {
    profile: CompileProfile,
    checkpoint_export: &'a std::path::Path,
    tokenizer: Option<&'a std::path::Path>,
    out: &'a std::path::Path,
    tokens: Option<u16>,
    top_k: Option<u8>,
    temperature: Option<f64>,
    rng_seed: Option<u16>,
}

#[cfg(feature = "compile")]
fn run_compile(invocation: CompileInvocation<'_>) -> Result<(), RunCompileError> {
    let CompileInvocation {
        profile,
        checkpoint_export,
        tokenizer,
        out,
        tokens,
        top_k,
        temperature,
        rng_seed,
    } = invocation;
    match profile {
        CompileProfile::DenseBigram => {
            use gbf_codegen::compile::{
                CompileOptions, compile_checkpoint_export, write_build_outputs,
            };
            if tokenizer.is_some() || top_k.is_some() || temperature.is_some() || rng_seed.is_some()
            {
                return Err(RunCompileError::Usage(
                    "--tokenizer, --top-k, --temperature, and --rng-seed require --profile interactive-subword-dmg"
                        .to_string(),
                ));
            }
            let n_tokens = tokens.unwrap_or(256);
            let compiled =
                compile_checkpoint_export(checkpoint_export, &CompileOptions { n_tokens })
                    .map_err(RunCompileError::Dense)?;
            let outputs = write_build_outputs(&compiled, out).map_err(RunCompileError::Dense)?;
            let rom = &compiled.report.rom;
            println!(
                "compiled {} -> {} ({} bytes, {} banks, {} weight chunks, {}-token loop)",
                checkpoint_export.display(),
                outputs.rom_path.display(),
                rom.rom_bytes,
                rom.bank_count,
                rom.weight_chunk_count,
                rom.n_tokens
            );
            println!(
                "artifact {} | build report {}",
                compiled.report.artifact.semantic_hash,
                outputs.report_path.display()
            );
        }
        CompileProfile::InteractiveSubwordDmg => {
            use gbf_codegen::compile_state_subword::{
                InteractiveSubwordCompileOptions, compile_interactive_subword,
                write_interactive_subword_outputs,
            };
            let tokenizer = tokenizer.ok_or_else(|| {
                RunCompileError::Usage(
                    "--tokenizer is required for --profile interactive-subword-dmg".to_string(),
                )
            })?;
            let n_tokens = u8::try_from(tokens.unwrap_or(24)).map_err(|_| {
                RunCompileError::Usage(
                    "interactive --tokens must fit the cartridge range 1..=255".to_string(),
                )
            })?;
            let options = InteractiveSubwordCompileOptions {
                n_tokens,
                top_k: top_k.unwrap_or(4),
                temperature: temperature.unwrap_or(0.6),
                rng_seed: rng_seed.unwrap_or(0x5EED),
            };
            let compiled = compile_interactive_subword(checkpoint_export, tokenizer, &options)
                .map_err(RunCompileError::Interactive)?;
            let outputs = write_interactive_subword_outputs(&compiled, out)
                .map_err(RunCompileError::Interactive)?;
            let rom = &compiled.report.rom;
            println!(
                "compiled {} + {} -> {} ({} bytes, {} banks, {} generated tokens)",
                checkpoint_export.display(),
                tokenizer.display(),
                outputs.rom_path.display(),
                rom.rom_bytes,
                rom.bank_count,
                rom.generation_tokens,
            );
            println!(
                "ROM sha256 {} | symbols {} | build report {} | compile request {}",
                rom.sha256,
                outputs.symbols_path.display(),
                outputs.report_path.display(),
                outputs.request_path.display(),
            );
        }
    }
    Ok(())
}

fn parse_u16(value: &str) -> Result<u16, String> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16).map_err(|error| error.to_string())
    } else {
        value.parse::<u16>().map_err(|error| error.to_string())
    }
}

fn exit_code<E: std::fmt::Display>(result: Result<(), E>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn exit_code_from_clap(kind: clap::error::ErrorKind) -> ExitCode {
    match kind {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
            ExitCode::SUCCESS
        }
        _ => ExitCode::from(2),
    }
}
