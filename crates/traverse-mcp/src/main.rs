use std::process::ExitCode;
use traverse_mcp::run_stdio_server;

fn main() -> ExitCode {
    run(std::env::args().skip(1))
}

/// Testable core of [`main`]: takes the argument iterator directly instead of
/// reading `std::env::args()` so the CLI parsing branches can be exercised
/// without spawning a subprocess.
fn run(mut args: impl Iterator<Item = String>) -> ExitCode {
    let Some(command) = args.next() else {
        eprintln!("Usage: traverse-mcp stdio [--simulate-startup-failure]");
        return ExitCode::from(1);
    };

    if command != "stdio" {
        eprintln!("Unsupported command: {command}");
        return ExitCode::from(1);
    }

    let simulate_startup_failure = args.any(|argument| argument == "--simulate-startup-failure");
    match run_stdio_server(simulate_startup_failure) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("traverse-mcp stdio server failed: {error:?}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_command_prints_usage_and_exits_with_failure() {
        assert_eq!(run(std::iter::empty()), ExitCode::from(1));
    }

    #[test]
    fn unsupported_command_exits_with_failure() {
        assert_eq!(run(["bogus".to_string()].into_iter()), ExitCode::from(1));
    }

    #[test]
    fn stdio_command_with_simulated_startup_failure_exits_with_failure() {
        assert_eq!(
            run([
                "stdio".to_string(),
                "--simulate-startup-failure".to_string()
            ]
            .into_iter()),
            ExitCode::from(1)
        );
    }
}
