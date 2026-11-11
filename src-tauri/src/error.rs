use serde::Serialize;
use std::{fmt, io, path::Path};

/// Machine readable error category. The frontend translates these codes,
/// so OS messages (which are always English) are only a fallback.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum ErrorCode {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    DirectoryNotEmpty,
    InvalidName,
    CrossesDevices,
    StorageFull,
    ReadOnly,
    /// macOS: an NTFS disk, which the system mounts read-only.
    ReadOnlyNtfs,
    Busy,
    Cancelled,
    IntoItself,
    VerifyFailed,
    Ftp,
    FtpConnect,
    FtpAuth,
    /// SSH server key not known yet (message = fingerprint to confirm).
    HostKeyUnknown,
    HostKeyChanged,
    /// adb / docker / rclone not installed.
    ToolMissing,
    VaultLocked,
    VaultWrongPassword,
    Keychain,
    Unsupported,
    InvalidInput,
    Archive,
    PasswordRequired,
    WrongPassword,
    Io,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), path: None }
    }

    pub fn with_path(mut self, path: impl AsRef<Path>) -> Self {
        self.path = Some(path.as_ref().to_string_lossy().into_owned());
        self
    }

    pub fn io(err: io::Error, path: impl AsRef<Path>) -> Self {
        let mut e = Self::from(err);
        if e.code == ErrorCode::ReadOnly && on_ntfs(path.as_ref()) {
            e.code = ErrorCode::ReadOnlyNtfs;
        }
        e.with_path(path)
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorCode::Cancelled, "Operation cancelled")
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, message)
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Other, message)
    }

    pub fn is_cancelled(&self) -> bool {
        self.code == ErrorCode::Cancelled
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(p) => write!(f, "{} ({})", self.message, p),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for AppError {}

/// Whether `path` (or its nearest existing ancestor) is on an NTFS volume.
#[cfg(target_os = "macos")]
fn on_ntfs(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Some(p) = path.ancestors().find(|p| p.exists()) else { return false };
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return false;
    }
    unsafe { std::ffi::CStr::from_ptr(st.f_fstypename.as_ptr()) }.to_bytes().eq_ignore_ascii_case(b"ntfs")
}

#[cfg(not(target_os = "macos"))]
fn on_ntfs(_: &Path) -> bool {
    false
}

pub fn code_for_io(kind: io::ErrorKind) -> ErrorCode {
    use io::ErrorKind as K;
    match kind {
        K::NotFound => ErrorCode::NotFound,
        K::PermissionDenied => ErrorCode::PermissionDenied,
        K::AlreadyExists => ErrorCode::AlreadyExists,
        K::NotADirectory => ErrorCode::NotADirectory,
        K::IsADirectory => ErrorCode::IsADirectory,
        K::DirectoryNotEmpty => ErrorCode::DirectoryNotEmpty,
        K::InvalidFilename => ErrorCode::InvalidName,
        K::CrossesDevices => ErrorCode::CrossesDevices,
        K::StorageFull | K::QuotaExceeded => ErrorCode::StorageFull,
        K::ReadOnlyFilesystem => ErrorCode::ReadOnly,
        K::ResourceBusy | K::ExecutableFileBusy => ErrorCode::Busy,
        K::Unsupported => ErrorCode::Unsupported,
        K::InvalidInput => ErrorCode::InvalidInput,
        _ => ErrorCode::Io,
    }
}

impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        Self::new(code_for_io(err.kind()), err.to_string())
    }
}

impl From<suppaftp::FtpError> for AppError {
    fn from(err: suppaftp::FtpError) -> Self {
        use suppaftp::FtpError as F;
        let code = match &err {
            F::ConnectionError(e) => match e.kind() {
                io::ErrorKind::NotFound => ErrorCode::NotFound,
                _ => ErrorCode::FtpConnect,
            },
            F::SecureError(_) | F::InvalidAddress(_) => ErrorCode::FtpConnect,
            F::UnexpectedResponse(r) => match r.status.code() {
                530 | 331 | 332 => ErrorCode::FtpAuth,
                550 => ErrorCode::NotFound,
                553 => ErrorCode::InvalidName,
                552 => ErrorCode::StorageFull,
                _ => ErrorCode::Ftp,
            },
            _ => ErrorCode::Ftp,
        };
        let message = match &err {
            F::UnexpectedResponse(r) => {
                format!("{} {}", r.status.code(), String::from_utf8_lossy(&r.body).trim())
            }
            other => other.to_string(),
        };
        Self::new(code, message)
    }
}

impl From<zip::result::ZipError> for AppError {
    fn from(err: zip::result::ZipError) -> Self {
        match err {
            zip::result::ZipError::Io(e) => e.into(),
            other => Self::new(ErrorCode::Archive, other.to_string()),
        }
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        Self::other(err.to_string())
    }
}
