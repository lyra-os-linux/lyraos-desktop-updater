//! Qualification adapter: only reads a private RPM root; never applies packages.
use lyra_upgrade_core::{ManifestChannelPolicy, ReleaseManifest, validate_manifest_route};
use lyra_upgrade_service::{migration, solver_xml, vendor_metadata};
use std::{fs, path::Path};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 5, "mode manifest private-root input");
    let manifest: ReleaseManifest = serde_json::from_slice(&fs::read(&args[2]).unwrap()).unwrap();
    validate_manifest_route(
        &manifest,
        &manifest.source,
        None,
        env!("CARGO_PKG_VERSION"),
        ManifestChannelPolicy::Testing,
    )
    .unwrap();
    let root = Path::new(&args[3]);
    assert_ne!(root, Path::new("/"), "private root required");
    let installed = vendor_metadata::installed_packages(Some(root)).unwrap();
    match args[1].as_str() {
        "arguments" => {
            let mode = match args[4].as_str() {
                "plan" => migration::TransactionMode::Plan,
                "download" => migration::TransactionMode::Download,
                "apply" => migration::TransactionMode::Apply,
                _ => panic!("unknown mode"),
            };
            println!(
                "{}",
                serde_json::to_string(
                    &migration::transaction_arguments(&manifest, &installed, mode).unwrap()
                )
                .unwrap()
            );
        }
        "validate" => {
            let mut solver = solver_xml::parse_solver_xml(
                &fs::read_to_string(&args[4]).unwrap(),
                vec!["fixture".into()],
                0,
            )
            .unwrap();
            vendor_metadata::enrich_solver_vendors_at(
                &mut solver,
                &root.join("var/cache/zypp/raw"),
                Some(root),
            )
            .unwrap();
            lyra_upgrade_core::migration::validate_migration_plan(
                &manifest,
                &installed,
                &solver.changes,
            )
            .unwrap();
            println!("{}", serde_json::to_string(&solver.changes).unwrap());
        }
        "payloads" => {
            migration::verify_payloads(&manifest, &installed, Path::new(&args[4])).unwrap()
        }
        _ => panic!("unknown action"),
    }
}
