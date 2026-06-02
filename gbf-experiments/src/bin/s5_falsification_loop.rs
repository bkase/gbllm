//! Run the active F-S5 `s5-falsify-N` feature-loop case.

use std::process::ExitCode;

use gbf_experiments::s5::falsify::run_active_s5_falsification_case;

fn main() -> ExitCode {
    let Some(result) = run_active_s5_falsification_case() else {
        eprintln!("exactly one s5-falsify-N feature must be enabled");
        return ExitCode::from(2);
    };

    match serde_json::to_string(&result) {
        Ok(json) if result.matches_expected => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Ok(json) => {
            eprintln!("{json}");
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("failed to serialize S5 falsification result: {error}");
            ExitCode::from(1)
        }
    }
}
