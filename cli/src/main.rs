use lyra_upgrade_core::{
    HostFacts, PreflightPolicy, PreflightReport, SystemBackend, discover_host, evaluate_preflight,
};
use serde::Serialize;

fn main() {
    if !valid_arguments(std::env::args().skip(1)) {
        eprintln!("usage: lyra-upgrade [inspect]");
        std::process::exit(2);
    }
    let facts = match discover_host(&SystemBackend) {
        Ok(facts) => facts,
        Err(error) => {
            eprintln!("host discovery failed: {error:?}");
            std::process::exit(1);
        }
    };
    let preflight = evaluate_preflight(&facts, PreflightPolicy::default());
    let inspection = Inspection { facts, preflight };
    println!(
        "{}",
        serde_json::to_string_pretty(&inspection).expect("serialize inspection")
    );
}

fn valid_arguments(arguments: impl IntoIterator<Item = String>) -> bool {
    let arguments: Vec<_> = arguments.into_iter().collect();
    arguments.is_empty() || matches!(arguments.as_slice(), [command] if command == "inspect")
}

#[derive(Serialize)]
struct Inspection {
    facts: HostFacts,
    preflight: PreflightReport,
}

#[cfg(test)]
mod tests {
    use super::valid_arguments;

    #[test]
    fn cli_accepts_only_default_or_inspect() {
        assert!(valid_arguments(Vec::<String>::new()));
        assert!(valid_arguments(["inspect".into()]));
        assert!(!valid_arguments(["apply".into()]));
        assert!(!valid_arguments(["inspect".into(), "extra".into()]));
    }
}
