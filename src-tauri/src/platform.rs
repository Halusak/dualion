//! Operating system integration: app info, opening files, terminals,
//! editors, Quick Look and native property dialogs.

use crate::error::{AppError, AppResult};
use crate::util::path_str;
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub key: &'static str,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub platform: &'static str,
    pub version: String,
    pub home: String,
    pub places: Vec<Place>,
    pub accent: Option<String>,
    pub locale: String,
    /// "vibrancy" | "mica" | "none"
    pub effects: &'static str,
    pub os_build: u32,
    pub separator: String,
}

pub fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

pub fn places() -> Vec<Place> {
    let mut v = Vec::new();
    let mut push = |key, p: Option<std::path::PathBuf>| {
        if let Some(p) = p.filter(|p| p.is_dir()) {
            v.push(Place { key, path: path_str(&p) });
        }
    };
    push("home", dirs::home_dir());
    push("desktop", dirs::desktop_dir());
    push("documents", dirs::document_dir());
    push("downloads", dirs::download_dir());
    push("pictures", dirs::picture_dir());
    push("music", dirs::audio_dir());
    push("videos", dirs::video_dir());
    if cfg!(target_os = "macos") {
        push("applications", Some("/Applications".into()));
    }
    v
}

pub fn accent_color() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        crate::icons::macos::accent_color()
    }
    #[cfg(windows)]
    {
        crate::icons::windows::accent_color()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

pub fn os_build() -> u32 {
    #[cfg(windows)]
    {
        crate::icons::windows::build_number()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

fn spawn(cmd: &mut Command) -> AppResult<()> {
    cmd.spawn().map(|_| ()).map_err(|e| AppError::other(format!("{e}")))
}

/// Opens a terminal in `dir`. `custom` may name an app (macOS: "iTerm",
/// "Warp"…) or a command line with `{dir}` placeholder.
pub fn open_terminal(dir: &str, custom: Option<&str>) -> AppResult<()> {
    let custom = custom.map(str::trim).filter(|c| !c.is_empty());
    if let Some(c) = custom {
        if c.contains("{dir}") {
            let line = c.replace("{dir}", dir);
            #[cfg(windows)]
            return spawn(Command::new("cmd").args(["/C", &line]).current_dir(dir));
            #[cfg(not(windows))]
            return spawn(Command::new("sh").args(["-c", &line]).current_dir(dir));
        }
    }
    #[cfg(target_os = "macos")]
    {
        spawn(Command::new("open").args(["-a", custom.unwrap_or("Terminal"), dir]))
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x10;
        if let Some(app) = custom {
            return spawn(Command::new(app).current_dir(dir).creation_flags(CREATE_NEW_CONSOLE));
        }
        if Command::new("wt.exe").args(["-d", dir]).spawn().is_ok() {
            return Ok(());
        }
        spawn(Command::new("cmd.exe").current_dir(dir).creation_flags(CREATE_NEW_CONSOLE))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        spawn(Command::new(custom.unwrap_or("x-terminal-emulator")).current_dir(dir))
    }
}

/// F4: opens a file in the configured editor or the system text editor.
pub fn edit_file(path: &str, editor: Option<&str>) -> AppResult<()> {
    if let Some(ed) = editor.map(str::trim).filter(|e| !e.is_empty()) {
        #[cfg(target_os = "macos")]
        if !ed.contains('/') {
            return spawn(Command::new("open").args(["-a", ed, path]));
        }
        return spawn(Command::new(ed).arg(path));
    }
    #[cfg(target_os = "macos")]
    {
        spawn(Command::new("open").args(["-t", path]))
    }
    #[cfg(windows)]
    {
        spawn(Command::new("notepad.exe").arg(path))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        spawn(Command::new("xdg-open").arg(path))
    }
}

pub fn open_with(path: &str, app: &str) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        spawn(Command::new("open").args(["-a", app, path]))
    }
    #[cfg(not(target_os = "macos"))]
    {
        spawn(Command::new(app).arg(path))
    }
}

pub fn quick_look(path: &str) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Stdio;
        spawn(Command::new("qlmanage").args(["-p", path]).stdout(Stdio::null()).stderr(Stdio::null()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err(AppError::unsupported("Quick Look is only available on macOS"))
    }
}

/// Shows the native Get Info / Properties window.
pub fn show_properties(path: &str) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "tell application \"Finder\"\nactivate\nopen information window of (POSIX file \"{escaped}\" as alias)\nend tell"
        );
        spawn(Command::new("osascript").args(["-e", &script]))
    }
    #[cfg(windows)]
    {
        let p = path.to_string();
        // SHObjectProperties needs a thread that keeps pumping; give it one.
        std::thread::spawn(move || {
            if crate::icons::windows::show_properties(&p) {
                std::thread::sleep(std::time::Duration::from_secs(600));
            }
        });
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
        Err(AppError::unsupported("Not supported"))
    }
}

pub fn temp_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join("dualion");
    let _ = std::fs::create_dir_all(&d);
    d
}
