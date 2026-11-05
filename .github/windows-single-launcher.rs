#![windows_subsystem = "windows"]

use std::{
    error::Error,
    ffi::OsString,
    fs,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const BUNDLE: &[u8] = include_bytes!(env!("HAGENCY_TEST_BUNDLE"));
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(window: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

fn run() -> Result<(), Box<dyn Error>> {
    let profile = match std::env::var_os("HAGENCY_DESKTOP_DATA_DIR") {
        Some(path) => PathBuf::from(path),
        None => {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
                .join("HagencyDesktop-WebView-Test")
        }
    };
    if !profile.is_absolute() {
        return Err("The test profile directory must be absolute".into());
    }
    fs::create_dir_all(&profile)?;

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary = std::env::temp_dir().join(format!(
        "hagency-webview-test-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&temporary)?;
    let result = (|| -> Result<(), Box<dyn Error>> {
        let archive = temporary.join("payload.zip");
        let unpacked = temporary.join("app");
        fs::write(&archive, BUNDLE)?;
        let powershell = PathBuf::from(
            std::env::var_os("SystemRoot").ok_or("Windows system directory is unavailable")?,
        )
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let status = Command::new(powershell)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
                "$ErrorActionPreference='Stop'; Expand-Archive -LiteralPath $env:HAGENCY_TEST_ARCHIVE -DestinationPath $env:HAGENCY_TEST_UNPACKED -Force"])
            .env("HAGENCY_TEST_ARCHIVE", &archive)
            .env("HAGENCY_TEST_UNPACKED", &unpacked)
            .creation_flags(CREATE_NO_WINDOW)
            .status()?;
        if !status.success() {
            return Err(format!("Could not unpack the test application ({status})").into());
        }
        let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
        let status = Command::new(unpacked.join("rinx.exe"))
            .args(arguments)
            .current_dir(&unpacked)
            .env("HAGENCY_DESKTOP_DATA_DIR", profile)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .creation_flags(CREATE_NO_WINDOW)
            .status()?;
        if !status.success() {
            return Err(format!("The test application exited with {status}").into());
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&temporary);
    result
}

fn main() {
    if let Err(error) = run() {
        let title: Vec<u16> = "Hagency Desktop test"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let text: Vec<u16> = format!("Could not start the test build:\n{error}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            MessageBoxW(0, text.as_ptr(), title.as_ptr(), 0x10);
        }
        std::process::exit(1);
    }
}
