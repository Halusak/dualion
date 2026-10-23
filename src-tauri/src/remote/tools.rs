//! Locating and running command line tools (adb, docker, rclone). Apps
//! started from Finder / the Start menu do not inherit the shell's PATH, so
//! the usual install locations are searched as well.

use crate::error::{AppError, AppResult, ErrorCode};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn candidates(tool: &str) -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut v: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        v.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
        match tool {
            "adb" => {
                v.push(home.join("Library/Android/sdk/platform-tools"));
                if let Ok(sdk) = std::env::var("ANDROID_HOME") {
                    v.push(PathBuf::from(sdk).join("platform-tools"));
                }
                // SDKs on other volumes (e.g. /Volumes/Data/android).
                if let Ok(rd) = std::fs::read_dir("/Volumes") {
                    for e in rd.flatten() {
                        v.push(e.path().join("android/platform-tools"));
                        v.push(e.path().join("Android/sdk/platform-tools"));
                    }
                }
            }
            "docker" => {
                v.push(PathBuf::from("/Applications/Docker.app/Contents/Resources/bin"));
                v.push(home.join(".docker/bin"));
                v.push(home.join(".rd/bin"));
                v.push(PathBuf::from("/Applications/OrbStack.app/Contents/MacOS/xbin"));
            }
            _ => {}
        }
    }
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
        let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
        match tool {
            "adb" => {
                v.push(local.join("Android\\Sdk\\platform-tools"));
                if let Ok(sdk) = std::env::var("ANDROID_HOME") {
                    v.push(PathBuf::from(sdk).join("platform-tools"));
                }
            }
            "docker" => v.push(pf.join("Docker\\Docker\\resources\\bin")),
            "podman" => v.push(pf.join("RedHat\\Podman")),
            "rclone" => {
                v.push(pf.join("rclone"));
                v.push(local.join("Microsoft\\WinGet\\Links"));
                v.push(home.join("scoop\\shims"));
            }
            _ => {}
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        v.extend(["/usr/local/bin", "/usr/bin", "/snap/bin", "/var/lib/flatpak/exports/bin"].map(PathBuf::from));
        if tool == "adb" {
            v.push(home.join("Android/Sdk/platform-tools"));
            if let Ok(sdk) = std::env::var("ANDROID_HOME") {
                v.push(PathBuf::from(sdk).join("platform-tools"));
            }
        }
    }
    v.into_iter().map(|d| d.join(exe(tool))).collect()
}

/// Full path of a tool, if installed. `DUALION_<TOOL>_BIN` overrides the search.
pub fn find(tool: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(format!("DUALION_{}_BIN", tool.to_ascii_uppercase())) {
        return Some(PathBuf::from(p));
    }
    candidates(tool).into_iter().find(|p| p.is_file())
}

static ADB: LazyLock<Option<PathBuf>> = LazyLock::new(|| find("adb"));
static RCLONE: LazyLock<Option<PathBuf>> = LazyLock::new(|| find("rclone"));
static CONTAINERS: LazyLock<Option<PathBuf>> = LazyLock::new(|| find("docker").or_else(|| find("podman")));

pub fn adb() -> AppResult<PathBuf> {
    ADB.clone().ok_or_else(|| missing("adb"))
}
pub fn rclone() -> AppResult<PathBuf> {
    RCLONE.clone().ok_or_else(|| missing("rclone"))
}
/// docker, or podman when Docker is not installed.
pub fn docker() -> AppResult<PathBuf> {
    CONTAINERS.clone().ok_or_else(|| missing("docker"))
}

fn missing(tool: &str) -> AppError {
    AppError::new(ErrorCode::ToolMissing, format!("{tool} is not installed"))
}

pub fn command(path: &PathBuf) -> Command {
    let mut c = Command::new(path);
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Runs a command, killing it after `timeout`. Fails with its stderr.
pub fn run(mut cmd: Command, timeout: Duration) -> AppResult<Output> {
    let mut child = cmd.spawn().map_err(|e| AppError::other(e.to_string()))?;
    // Read output on threads so a chatty tool cannot block on a full pipe.
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = std::io::Read::read_to_end(o, &mut b);
        }
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(e) = err.as_mut() {
            let _ = std::io::Read::read_to_end(e, &mut b);
        }
        b
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| AppError::other(e.to_string()))? {
            break s;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::new(ErrorCode::Io, "The command timed out"));
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    let output = Output { status, stdout: t_out.join().unwrap_or_default(), stderr: t_err.join().unwrap_or_default() };
    if !output.status.success() {
        return Err(tool_error(&output.stderr));
    }
    Ok(output)
}

/// Maps a tool's error text to an error code (not found, permission…).
pub fn tool_error(stderr: &[u8]) -> AppError {
    let msg = String::from_utf8_lossy(stderr).trim().to_string();
    let lower = msg.to_lowercase();
    let code = if lower.contains("no such file") || lower.contains("not found") || lower.contains("directory not found") || lower.contains("object not found") {
        ErrorCode::NotFound
    } else if lower.contains("permission denied") || lower.contains("read-only") || lower.contains("operation not permitted") {
        ErrorCode::PermissionDenied
    } else if lower.contains("file exists") || lower.contains("already exists") {
        ErrorCode::AlreadyExists
    } else if lower.contains("not empty") {
        ErrorCode::DirectoryNotEmpty
    } else {
        ErrorCode::Io
    };
    AppError::new(code, if msg.is_empty() { "The command failed".to_string() } else { msg.lines().last().unwrap_or(&msg).to_string() })
}
