//! Bound read-only probes, including their subprocesses and captured output.
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub fn output(command: &mut Command, seconds: u64) -> io::Result<Output> {
    const LIMIT: u64 = 32 * 1024 * 1024;
    let mut child = command
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = child.id() as i32;
    let capture = |stream: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut data = Vec::new();
            stream.take(LIMIT + 1).read_to_end(&mut data).map(|_| data)
        })
    };
    let stdout = capture(Box::new(
        child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("stdout unavailable"))?,
    ));
    let stderr = capture(Box::new(
        child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("stderr unavailable"))?,
    ));
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Descendants may retain a pipe after the leader exited. Never wait on them
    // indefinitely; no command issued by a preview should leave daemons behind.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.wait();
    let stdout = stdout
        .join()
        .map_err(|_| io::Error::other("stdout worker"))??;
    let stderr = stderr
        .join()
        .map_err(|_| io::Error::other("stderr worker"))??;
    let status =
        status.ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "probe timed out"))?;
    if stdout.len() as u64 > LIMIT || stderr.len() as u64 > LIMIT {
        return Err(io::Error::other("probe output too large"));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sleeping_probe_is_bounded_and_ordinary_output_is_preserved() {
        let started = Instant::now();
        let error = output(Command::new("sleep").arg("30"), 0).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        let result = output(Command::new("printf").arg("probe-ok"), 2).unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, b"probe-ok");
    }
}
