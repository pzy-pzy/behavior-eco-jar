//! 本地日志：只记生命周期与错误，不记应用名、不记窗口标题。
//!
//! 位置：<data>/ecojar.log，超过 1MB 轮转一次，只留一份旧档。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init(dir: &Path) {
    let _ = fs::create_dir_all(dir);
    let path = dir.join("ecojar.log");
    if fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = fs::remove_file(dir.join("ecojar.log.1"));
        let _ = fs::rename(&path, dir.join("ecojar.log.1"));
    }
    let _ = LOG_PATH.set(path);

    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "?".into());
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };
        write(&format!("PANIC {msg} @ {location}"));
        eprintln!("PANIC {msg} @ {location}");
    }));
}

pub fn write(msg: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    let line = format!("{} {}\n", timestamp(), msg);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn path_string() -> String {
    LOG_PATH
        .get()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(windows)]
fn timestamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = unsafe { std::mem::zeroed::<windows_sys::Win32::Foundation::SYSTEMTIME>() };
    unsafe { GetLocalTime(&mut t) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

#[cfg(not(windows))]
fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
