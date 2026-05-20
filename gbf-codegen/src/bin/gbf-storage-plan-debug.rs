use gbf_codegen::storage_plan_test_infra::debug_harness;

fn main() {
    let command_line: Vec<String> = std::env::args().collect();
    let args = match debug_harness::parse_args(command_line.iter().skip(1).cloned()) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    match debug_harness::run(args, command_line) {
        Ok(status) => std::process::exit(status),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}
