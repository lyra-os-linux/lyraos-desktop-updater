#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use lyra_upgrade_protocol::Response;
use lyra_upgrade_service::planner::plan_update_with_cached_metadata;

struct ServiceProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Default)]
struct ServiceClient(Mutex<Option<ServiceProcess>>);

#[tauri::command]
fn layout_preview_enabled() -> bool {
    std::env::var_os("LYRA_UPGRADE_LAYOUT_PREVIEW").is_some()
}

/// The GNOME appearance the desktop is set to, so the window can paint light or
/// dark instead of being dark-only. Same schema and key that Vega's appearance
/// module and Lyra Welcome read, so the three never disagree. The stylesheet
/// falls back to prefers-color-scheme when this returns nothing, which is what
/// a non-GNOME session or a missing gsettings leaves behind.
#[tauri::command]
fn color_scheme() -> String {
    let output = Command::new("/usr/bin/gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(result) if result.status.success() => {
            match String::from_utf8_lossy(&result.stdout)
                .trim()
                .trim_matches('\'')
                .trim_matches('"')
            {
                "prefer-dark" => "dark".to_owned(),
                // "default" means no stated preference, which GNOME renders light.
                _ => "light".to_owned(),
            }
        }
        _ => "unknown".to_owned(),
    }
}

#[tauri::command]
async fn plan_update(
    request_id: String,
    operation_id: String,
) -> Result<serde_json::Value, String> {
    let planned = tauri::async_runtime::spawn_blocking(plan_update_with_cached_metadata)
        .await
        .map_err(|_| "PREFLIGHT_BLOCKED".to_string())?
        .map_err(|_| "PREFLIGHT_BLOCKED".to_string())?;
    serde_json::to_value(Response::Plan {
        request_id,
        operation_id,
        plan_sha256: planned.plan_sha256.clone(),
        plan: Box::new(planned.plan.clone()),
        preflight: planned.preflight.clone(),
        planned: Box::new(planned),
    })
    .map_err(|_| "PLAN_SERIALIZATION_FAILED".to_string())
}

#[tauri::command]
fn reboot_system() -> Result<(), String> {
    Command::new("systemctl")
        .arg("reboot")
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "REBOOT_FAILED".to_string())?
        .success()
        .then_some(())
        .ok_or_else(|| "REBOOT_FAILED".to_string())
}

#[tauri::command]
fn service_request(
    request: serde_json::Value,
    client: tauri::State<'_, ServiceClient>,
) -> Result<serde_json::Value, String> {
    let mut guard = client.0.lock().map_err(|_| "service client lock failed")?;
    if guard
        .as_mut()
        .is_some_and(|process| process.child.try_wait().ok().flatten().is_some())
    {
        *guard = None;
    }
    if guard.is_none() {
        let mut child = Command::new("pkexec")
            .arg("/usr/libexec/lyra-upgrade-service")
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot start service: {error}"))?;
        let stdin = child.stdin.take().ok_or("service stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("service stdout unavailable")?;
        *guard = Some(ServiceProcess {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        });
    }
    let process = guard.as_mut().ok_or("service unavailable")?;
    serde_json::to_writer(&mut process.stdin, &request)
        .map_err(|error| format!("cannot serialize request: {error}"))?;
    process
        .stdin
        .write_all(b"\n")
        .map_err(|error| error.to_string())?;
    process.stdin.flush().map_err(|error| error.to_string())?;
    let mut response = String::new();
    if process
        .stdout
        .read_line(&mut response)
        .map_err(|error| error.to_string())?
        == 0
    {
        if process
            .child
            .try_wait()
            .ok()
            .flatten()
            .and_then(|status| status.code())
            .is_some_and(|code| matches!(code, 126 | 127))
        {
            return Err("AUTHORIZATION".into());
        }
        return Err("service closed the protocol stream".into());
    }
    serde_json::from_str(&response).map_err(|error| format!("invalid service response: {error}"))
}

fn main() {
    tauri::Builder::default()
        .manage(ServiceClient::default())
        .invoke_handler(tauri::generate_handler![
            service_request,
            layout_preview_enabled,
            color_scheme,
            plan_update,
            reboot_system
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Lyra Upgrade");
}
