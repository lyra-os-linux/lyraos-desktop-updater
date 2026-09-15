//! Read-only local qualification: no authentication or package application.
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--diagnose-simulation") {
        let simulation = lyra_upgrade_service::simulation::Simulation::new(true).unwrap();
        for args in [
            vec!["--non-interactive", "--no-refresh", "lr", "--details"],
            vec!["--non-interactive", "--no-refresh", "locks"],
            vec![
                "--non-interactive",
                "--no-refresh",
                "packages",
                "--orphaned",
            ],
        ] {
            let out = simulation.context.output(&args).unwrap();
            eprintln!(
                "{args:?}: {:?}\n{}\n{}",
                out.status.code(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        return;
    }
    match lyra_upgrade_service::planner::plan_update_with_cached_metadata() {
        Ok(plan) => println!("{}", serde_json::to_string_pretty(&plan).unwrap()),
        Err(error) => {
            eprintln!("{error:?}");
            std::process::exit(1);
        }
    }
}
