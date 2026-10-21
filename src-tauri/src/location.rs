use serde::ser::SerializeTuple;
use serde::{Deserialize, Serialize, Serializer};

/// Where a panel points to. Serialized as `{ kind: "local", path }` etc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Location {
    Local { path: String },
    /// A connected remote file system (FTP, SFTP, WebDAV, S3, ADB, Docker, rclone).
    Remote { session: String, path: String },
    Archive { archive: String, inner: String },
    /// "This PC" – list of drives (Windows) / volumes (macOS).
    Drives,
    /// Computers on the local network, or the shared folders of `host`.
    Network {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
    },
}

impl Location {
    pub fn local(path: impl Into<String>) -> Self {
        Location::Local { path: path.into() }
    }
}

pub mod flags {
    pub const DIR: u32 = 1;
    pub const SYMLINK: u32 = 2;
    pub const HIDDEN: u32 = 4;
    pub const READONLY: u32 = 8;
    #[allow(dead_code)] // set on Windows only
    pub const SYSTEM: u32 = 16;
    pub const EXEC: u32 = 32;
    pub const BROKEN: u32 = 64;
    /// macOS bundle (.app, .photoslibrary…): a directory shown as a file.
    #[allow(dead_code)] // set on macOS only
    pub const PACKAGE: u32 = 128;
    /// Entry of the drives list.
    pub const VOLUME: u32 = 256;
    /// Cloud placeholder: content is online only (not downloaded).
    pub const CLOUD: u32 = 512;
}

/// One directory entry. Serialized as a compact tuple
/// `[name, flags, size, mtime, attrs]` – listings of 100k files stay small.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub flags: u32,
    pub size: u64,
    /// Milliseconds since epoch, 0 when unknown.
    pub mtime: i64,
    /// Unix mode bits or Windows file attributes.
    pub attrs: u32,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.flags & flags::DIR != 0
    }
}

impl Serialize for Entry {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut t = s.serialize_tuple(5)?;
        t.serialize_element(&self.name)?;
        t.serialize_element(&self.flags)?;
        t.serialize_element(&self.size)?;
        t.serialize_element(&self.mtime)?;
        t.serialize_element(&self.attrs)?;
        t.end()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crumb {
    pub label: String,
    /// "drives" | "volume" | "home" | "server" | "archive" | "folder"
    pub icon: &'static str,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub location: Location,
    /// Human readable path for the address bar / window title.
    pub display_path: String,
    pub parent: Option<Location>,
    pub crumbs: Vec<Crumb>,
    pub entries: Vec<Entry>,
}
