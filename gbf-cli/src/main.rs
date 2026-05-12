use std::process::ExitCode;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    /// S1 First Pulse experiment workflows.
    S1 {
        #[command(subcommand)]
        command: gbf_experiments::s1::cli::S1Command,
    },
    /// S2 QAT-survives experiment workflows.
    S2 {
        #[command(subcommand)]
        command: gbf_experiments::s2::cli::S2Command,
    },
}

fn main() -> ExitCode {
    match GbfCli::try_parse() {
        Ok(cli) => {
            let s2_logging = s2_logging(&cli);
            match cli.command {
                GbfCommand::S1 { command } => exit_code(gbf_experiments::s1::cli::run(
                    gbf_experiments::s1::cli::S1Cli { command },
                )),
                GbfCommand::S2 { command } => exit_code(gbf_experiments::s2::cli::run(
                    gbf_experiments::s2::cli::S2Cli {
                        command,
                        logging: s2_logging,
                    },
                )),
            }
        }
        Err(error) => {
            let _ = error.print();
            exit_code_from_clap(error.kind())
        }
    }
}

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
