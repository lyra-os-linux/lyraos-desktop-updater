//! Only private status and the global replay counter require this read broker.
use lyra_upgrade_protocol::{Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub const SOCKET: &str = "/run/lyra-upgrade-query.socket";
pub const MAX_REQUEST: u64 = 4 * 1024 * 1024;
pub const MAX_RESPONSE: u64 = 16 * 1024 * 1024;

pub fn peer_uid(fd: RawFd) -> std::io::Result<u32> {
    let mut credential: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credential as *mut libc::ucred).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(std::io::Error::other("peer credentials unavailable"));
    }
    Ok(credential.uid)
}

pub fn request(request: &Request) -> Result<Response, String> {
    use std::io::Read;
    let mut stream = UnixStream::connect(SOCKET).map_err(|_| "QUERY_UNAVAILABLE")?;
    if peer_uid(stream.as_raw_fd()).map_err(|_| "QUERY_UNAVAILABLE")? != 0 {
        return Err("QUERY_UNAVAILABLE".into());
    }
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "QUERY_UNAVAILABLE")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "QUERY_UNAVAILABLE")?;
    serde_json::to_writer(&mut stream, request).map_err(|_| "INVALID_REQUEST")?;
    stream.write_all(b"\n").map_err(|_| "QUERY_UNAVAILABLE")?;
    let mut reply = Vec::new();
    BufReader::new(stream.take(MAX_RESPONSE + 1))
        .read_until(b'\n', &mut reply)
        .map_err(|_| "QUERY_UNAVAILABLE")?;
    if reply.len() as u64 > MAX_RESPONSE || reply.last() != Some(&b'\n') {
        return Err("INVALID_RESPONSE".into());
    }
    serde_json::from_slice(&reply).map_err(|_| "INVALID_RESPONSE".into())
}
