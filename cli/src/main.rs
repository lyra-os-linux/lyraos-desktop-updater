use lyra_upgrade_core::{
    HostFacts, PreflightPolicy, PreflightReport, SystemBackend, discover_host, evaluate_preflight,
};
use serde::Serialize;

fn main() {
    if std::env::args()
        .nth(1)
        .as_deref()
        .is_some_and(|arg| arg != "inspect")
    {
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

#[derive(Serialize)]
struct Inspection {
    facts: HostFacts,
    preflight: PreflightReport,
}
