use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "gbf", about = "GBLLM command-line tools")]
struct GbfCli {
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
}

fn main() -> ExitCode {
    match GbfCli::try_parse() {
        Ok(cli) => match cli.command {
            GbfCommand::S1 { command } => exit_code(gbf_experiments::s1::cli::run(
                gbf_experiments::s1::cli::S1Cli { command },
            )),
        },
        Err(error) => {
            let _ = error.print();
            exit_code_from_clap(error.kind())
        }
    }
}

fn exit_code(result: Result<(), gbf_experiments::s1::cli::S1CliError>) -> ExitCode {
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
