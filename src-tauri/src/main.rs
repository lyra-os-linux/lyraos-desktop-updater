#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use lyra_upgrade_protocol::Request;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

struct ServiceProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Default)]
struct Connections {
    query: Option<ServiceProcess>,
    admin: Option<ServiceProcess>,
}
#[derive(Default)]
struct ServiceClient(Arc<Mutex<Connections>>);

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
async fn service_request(
    request: serde_json::Value,
    client: tauri::State<'_, ServiceClient>,
) -> Result<serde_json::Value, String> {
    let request: Request = serde_json::from_value(request).map_err(|_| "INVALID_REQUEST")?;
    if !request.is_supported() {
        return Err("UNSUPPORTED_PROTOCOL".into());
    }
    if matches!(
        request,
        Request::Start {
            confirmed: false,
            ..
        }
    ) {
        return Err("CONFIRMATION_REQUIRED".into());
    }
    let connections = client.0.clone();
    tauri::async_runtime::spawn_blocking(move || exchange(request, connections))
        .await
        .map_err(|_| "SERVICE_UNAVAILABLE".to_string())?
}

fn exchange(
    request: Request,
    connections: Arc<Mutex<Connections>>,
) -> Result<serde_json::Value, String> {
    let mut connections = connections.lock().map_err(|_| "SERVICE_UNAVAILABLE")?;
    let administrative = request.needs_authorization();
    let guard = if administrative {
        &mut connections.admin
    } else {
        &mut connections.query
    };
    if guard
        .as_mut()
        .is_some_and(|process| process.child.try_wait().ok().flatten().is_some())
    {
        *guard = None;
    }
    if guard.is_none() {
        let mut command = if administrative {
            let mut command = Command::new("/usr/bin/pkexec");
            command.arg("/usr/libexec/lyra-upgrade-service");
            command
        } else {
            let mut command = Command::new("/usr/libexec/lyra-upgrade-service");
            command.arg("--read-only");
            command
        };
        command
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin");
        if !administrative {
            for key in ["HOME", "XDG_CACHE_HOME"] {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
        }
        let mut child = command
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
    use std::io::Read;
    let mut response = String::new();
    if process
        .stdout
        .by_ref()
        .take(16 * 1024 * 1024 + 1)
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
    if response.len() > 16 * 1024 * 1024 || !response.ends_with('\n') {
        return Err("INVALID_RESPONSE".into());
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
            reboot_system
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Lyra Upgrade");
}
