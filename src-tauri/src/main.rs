#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

struct ServiceProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Default)]
struct ServiceClient(Mutex<Option<ServiceProcess>>);

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
        return Err("service closed the protocol stream".into());
    }
    serde_json::from_str(&response).map_err(|error| format!("invalid service response: {error}"))
}

fn main() {
    tauri::Builder::default()
        .manage(ServiceClient::default())
        .invoke_handler(tauri::generate_handler![service_request])
        .run(tauri::generate_context!())
        .expect("failed to run Lyra Upgrade");
}
