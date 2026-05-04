#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use gbf_debug::{
    ErrorEnvelope, ExecArgs, InitArgs, InspectArgs, ScriptConfig, run_exec, run_init, run_inspect,
};
use gbf_emu::{CycleBudget, MCycles};

#[derive(Debug, Parser)]
#[command(name = "gbf-debug", color = clap::ColorChoice::Never)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init {
        #[arg(long)]
        rom: PathBuf,
        #[arg(long)]
        sym: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1024)]
        trace_capacity: u32,
        #[arg(long, default_value_t = false)]
        replace_existing_out: bool,
    },
    Exec {
        #[arg(long = "in")]
        in_path: PathBuf,
        #[arg(long)]
        script: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 30)]
        timeout: u64,
        #[arg(long, default_value_t = 1_000_000)]
        default_run_m_cycles: u64,
        #[arg(long, default_value_t = 1_000_000)]
        max_step_instructions_per_call: u32,
        #[arg(long, default_value_t = false)]
        emit_metrics: bool,
        #[arg(long, default_value_t = false)]
        write_partial_on_timeout: bool,
        #[arg(long, default_value_t = false)]
        replace_existing_out: bool,
    },
    Inspect {
        session: PathBuf,
    },
}

fn main() -> ExitCode {
    let parsed = match Args::try_parse() {
        Ok(parsed) => parsed,
        Err(error) => {
            if error.exit_code() == 0 {
                let envelope = ErrorEnvelope::help(error.to_string());
                let mut stream = std::io::stdout();
                let _ = serde_json::to_writer(&mut stream, &envelope);
                let _ = writeln_no_fail(&mut stream);
                return ExitCode::SUCCESS;
            } else {
                let envelope = ErrorEnvelope::cli_args(error.to_string());
                let mut stream = std::io::stderr();
                let _ = serde_json::to_writer(&mut stream, &envelope);
                let _ = writeln_no_fail(&mut stream);
                return ExitCode::from(1);
            }
        }
    };

    let (command_name, result) = match parsed.command {
        Command::Init {
            rom,
            sym,
            out,
            trace_capacity,
            replace_existing_out,
        } => (
            "init",
            run_init(InitArgs {
                rom_path: rom,
                sym_path: sym,
                out_path: out,
                trace_capacity,
                replace_existing_out,
            })
            .map(|envelope| serde_json::to_value(envelope).expect("envelope serializes")),
        ),
        Command::Exec {
            in_path,
            script,
            out,
            timeout,
            default_run_m_cycles,
            max_step_instructions_per_call,
            emit_metrics,
            write_partial_on_timeout,
            replace_existing_out,
        } => (
            "exec",
            run_exec(ExecArgs {
                in_path,
                script_path: script,
                out_path: out,
                config: ScriptConfig {
                    timeout: Duration::from_secs(timeout),
                    default_run_budget: CycleBudget::Machine(MCycles(default_run_m_cycles)),
                    max_step_instructions_per_call,
                    ..ScriptConfig::default()
                },
                emit_metrics,
                write_partial_on_timeout,
                replace_existing_out,
            })
            .map(|envelope| serde_json::to_value(envelope).expect("envelope serializes")),
        ),
        Command::Inspect { session } => (
            "inspect",
            run_inspect(InspectArgs { in_path: session })
                .map(|envelope| serde_json::to_value(envelope).expect("envelope serializes")),
        ),
    };

    match result {
        Ok(value) => {
            let _ = serde_json::to_writer(std::io::stdout(), &value);
            let _ = writeln_no_fail(&mut std::io::stdout());
            ExitCode::SUCCESS
        }
        Err(error) => {
            let code = error.exit_code();
            let envelope = ErrorEnvelope::from_cli_error(command_name, &error);
            let _ = serde_json::to_writer(std::io::stderr(), &envelope);
            let _ = writeln_no_fail(&mut std::io::stderr());
            ExitCode::from(code)
        }
    }
}

fn writeln_no_fail(stream: &mut dyn std::io::Write) -> std::io::Result<()> {
    writeln!(stream)
}
