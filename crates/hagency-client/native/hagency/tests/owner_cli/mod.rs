#![allow(dead_code)]
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
pub fn command() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hagency-owner-test-host"));
    c.env("PATH", "").stdin(Stdio::null());
    c
}
pub struct Running {
    pub child: Child,
    pub address: SocketAddr,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    cookie: Option<&str>,
) -> String {
    request_result(address, method, path, body, cookie).unwrap()
}
fn request_result(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    cookie: Option<&str>,
) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let cookie = cookie
        .map(|value| format!("Cookie: {value}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nSec-Fetch-Site: same-origin\r\nContent-Type: application/json\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut out = String::new();
    if let Err(error) = stream.read_to_string(&mut out) {
        // An early authorization refusal may close with unread request bytes,
        // producing TCP RST after a complete response. Never accept truncation.
        let complete = out.split_once("\r\n\r\n").is_some_and(|(headers, body)| {
            headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .is_some_and(|length| body.len() == length)
        });
        if error.kind() != std::io::ErrorKind::ConnectionReset || !complete {
            return Err(error);
        }
    }
    Ok(out)
}
pub fn status(wire: &str) -> u16 {
    wire.lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}
pub fn launch(state: &Path) -> Running {
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut c = command();
    c.arg("serve")
        .arg("--state-dir")
        .arg(state)
        .args(["--listen", &address.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut run = Running {
        child: c.spawn().unwrap(),
        address,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(code) = run.child.try_wait().unwrap() {
            let mut stderr = String::new();
            run.child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("OwnerHost exited {code}: {stderr}");
        }
        // A successful TCP probe can precede HTTP readiness. Only this
        // side-effect-free health read retries within the existing deadline.
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok()
            && request_result(address, "GET", "/health", "", None).is_ok_and(|wire| {
                wire.lines()
                    .next()
                    .is_some_and(|line| line.split_whitespace().nth(1) == Some("200"))
            })
        {
            return run;
        }
        assert!(Instant::now() < deadline, "OwnerHost readiness timeout");
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub fn marker(state: &Path) -> Vec<u8> {
    fs::read(state.join("hagency-client-owned-v1.json")).unwrap()
}
#[cfg(unix)]
pub fn stop_clean(run: &mut Running) {
    let output = Command::new("/bin/kill")
        .args(["-TERM", &run.child.id().to_string()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(code) = run.child.try_wait().unwrap() {
            assert!(code.success(), "OwnerHost graceful exit: {code}");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "TERM exceeded service stop budget"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
