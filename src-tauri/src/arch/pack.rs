//! Creating archives: ZIP (optionally AES-256), 7z (LZMA2, optionally
//! AES-256 with encrypted names) and TAR (plain, gz, bz2, xz, zst).

use super::Comp;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::util::{self, system_time_ms};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    SevenZ,
    Tar(Comp),
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        Some(match s {
            "zip" => Format::Zip,
            "7z" => Format::SevenZ,
            "tar" => Format::Tar(Comp::None),
            "tar.gz" | "tgz" => Format::Tar(Comp::Gz),
            "tar.bz2" => Format::Tar(Comp::Bz2),
            "tar.xz" => Format::Tar(Comp::Xz),
            "tar.zst" => Format::Tar(Comp::Zst),
            _ => return None,
        })
    }

    /// Format implied by an archive file name.
    pub fn of_path(p: &Path) -> Option<Format> {
        match super::kind_of(&p.to_string_lossy())? {
            super::Kind::Zip => Some(Format::Zip),
            super::Kind::SevenZ => Some(Format::SevenZ),
            super::Kind::Tar(c) => Some(Format::Tar(c)),
            _ => None,
        }
    }
}

/// One file or folder to pack: (source, name in the archive, is_dir, size).
pub type Item = (PathBuf, String, bool, u64);

/// Walks the sources; names are relative to each source's parent.
pub fn collect(sources: &[PathBuf], skip: &Path) -> AppResult<Vec<Item>> {
    let mut items = Vec::new();
    for src in sources {
        let base = src.parent().map(Path::to_path_buf).unwrap_or_default();
        let walker = ignore::WalkBuilder::new(src).standard_filters(false).hidden(false).follow_links(false).build();
        for entry in walker.flatten() {
            let p = entry.path().to_path_buf();
            if p == skip {
                continue;
            }
            let rel = p.strip_prefix(&base).unwrap_or(&p);
            let name = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let size = if is_dir { 0 } else { entry.metadata().map(|m| m.len()).unwrap_or(0) };
            items.push((p, name, is_dir, size));
        }
    }
    Ok(items)
}

/// Counts bytes and checks for cancellation while a file is read.
struct Progress<'a, R> {
    inner: R,
    progress: &'a dyn Fn(u64) -> AppResult<()>,
}

impl<R: Read> Read for Progress<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            (self.progress)(n as u64).map_err(|e| std::io::Error::other(e.message))?;
        }
        Ok(n)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn pack(
    target: &Path,
    format: Format,
    items: &[Item],
    level: u8,
    password: Option<&str>,
    encrypt_names: bool,
    progress: &dyn Fn(u64) -> AppResult<()>,
    file_done: &mut dyn FnMut(&Item) -> AppResult<()>,
) -> AppResult<()> {
    let password = password.filter(|p| !p.is_empty());
    match format {
        Format::Zip => zip(target, items, level, password, progress, file_done),
        Format::SevenZ => sevenz(target, items, level, password, encrypt_names, progress, file_done),
        Format::Tar(c) => {
            if password.is_some() {
                return Err(AppError::unsupported("TAR archives cannot be encrypted – use ZIP or 7z"));
            }
            tar(target, c, items, level, progress, file_done)
        }
    }
}

fn zip(target: &Path, items: &[Item], level: u8, password: Option<&str>, progress: &dyn Fn(u64) -> AppResult<()>, file_done: &mut dyn FnMut(&Item) -> AppResult<()>) -> AppResult<()> {
    use zip::write::SimpleFileOptions;
    let file = File::create(target).map_err(|e| AppError::io(e, target))?;
    let mut zip = zip::ZipWriter::new(BufWriter::with_capacity(1 << 20, file));
    let level = level.min(9);
    let base = if level == 0 {
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)
    } else {
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(level as i64))
    };
    for it in items {
        let (p, name, is_dir, size) = it;
        let meta = fs::symlink_metadata(p).ok();
        let mut opts = base.large_file(*size >= 0xFFFF_FFFF);
        if let Some(m) = &meta {
            if let Ok(t) = m.modified() {
                let (y, mo, d, h, mi, s) = util::local_parts(system_time_ms(t));
                if let Ok(dt) = zip::DateTime::from_date_and_time(y.clamp(1980, 2107) as u16, mo as u8, d as u8, h as u8, mi as u8, s as u8) {
                    opts = opts.last_modified_time(dt);
                }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                opts = opts.unix_permissions(m.permissions().mode() & 0o7777);
            }
        }
        if *is_dir {
            zip.add_directory(format!("{name}/"), opts)?;
            continue;
        }
        let f = File::open(p).map_err(|e| AppError::io(e, p))?;
        match password {
            Some(pw) => zip.start_file(name.clone(), opts.with_aes_encryption(zip::AesMode::Aes256, pw))?,
            None => zip.start_file(name.clone(), opts)?,
        }
        std::io::copy(&mut Progress { inner: f, progress }, &mut zip).map_err(|e| io_or_cancel(e, p))?;
        file_done(it)?;
    }
    zip.finish()?.flush().map_err(|e| AppError::io(e, target))?;
    Ok(())
}

fn io_or_cancel(e: std::io::Error, p: &Path) -> AppError {
    if e.kind() == std::io::ErrorKind::Other && e.to_string().to_lowercase().contains("cancel") {
        return AppError::cancelled();
    }
    AppError::io(e, p)
}

#[allow(clippy::too_many_arguments)]
fn sevenz(
    target: &Path,
    items: &[Item],
    level: u8,
    password: Option<&str>,
    encrypt_names: bool,
    progress: &dyn Fn(u64) -> AppResult<()>,
    file_done: &mut dyn FnMut(&Item) -> AppResult<()>,
) -> AppResult<()> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    let err = |e: sevenz_rust2::Error| match e {
        sevenz_rust2::Error::Io(io, _) => io_or_cancel(io, target),
        other => AppError::new(ErrorCode::Archive, other.to_string()).with_path(target),
    };
    let mut w = sevenz_rust2::ArchiveWriter::create(target).map_err(err)?;
    let mut methods = Vec::new();
    if let Some(pw) = password {
        methods.push(AesEncoderOptions::new(sevenz_rust2::Password::new(pw)).into());
        w.set_encrypt_header(encrypt_names);
    }
    methods.push(Lzma2Options::from_level(level.min(9) as u32).into());
    w.set_content_methods(methods);
    for it in items {
        let (p, name, is_dir, _) = it;
        let entry = sevenz_rust2::ArchiveEntry::from_path(p, name.clone());
        if *is_dir {
            w.push_archive_entry::<File>(entry, None).map_err(err)?;
            continue;
        }
        let f = File::open(p).map_err(|e| AppError::io(e, p))?;
        w.push_archive_entry(entry, Some(Progress { inner: f, progress })).map_err(err)?;
        file_done(it)?;
    }
    w.finish().map_err(|e| AppError::io(e, target))?;
    Ok(())
}

fn tar(target: &Path, c: Comp, items: &[Item], level: u8, progress: &dyn Fn(u64) -> AppResult<()>, file_done: &mut dyn FnMut(&Item) -> AppResult<()>) -> AppResult<()> {
    let file = BufWriter::with_capacity(1 << 20, File::create(target).map_err(|e| AppError::io(e, target))?);
    let level = level.clamp(1, 9) as u32;
    let out: Box<dyn Write> = match c {
        Comp::None => Box::new(file),
        Comp::Gz => Box::new(flate2::write::GzEncoder::new(file, flate2::Compression::new(level))),
        Comp::Bz2 => Box::new(bzip2::write::BzEncoder::new(file, bzip2::Compression::new(level))),
        Comp::Xz => Box::new(lzma_rust2::XzWriter::new(file, lzma_rust2::XzOptions::with_preset(level)).map_err(|e| AppError::io(e, target))?.auto_finish()),
        Comp::Zst => Box::new(zstd::stream::write::Encoder::new(file, (level * 2) as i32).map_err(|e| AppError::io(e, target))?.auto_finish()),
    };
    let mut b = tar::Builder::new(out);
    b.follow_symlinks(false);
    for it in items {
        let (p, name, is_dir, _) = it;
        let md = fs::symlink_metadata(p).map_err(|e| AppError::io(e, p))?;
        let mut h = tar::Header::new_gnu();
        h.set_metadata(&md);
        if *is_dir {
            b.append_data(&mut h, format!("{name}/"), std::io::empty()).map_err(|e| AppError::io(e, target))?;
            continue;
        }
        if md.file_type().is_symlink() {
            let to = fs::read_link(p).map_err(|e| AppError::io(e, p))?;
            b.append_link(&mut h, name, to).map_err(|e| AppError::io(e, target))?;
            continue;
        }
        let f = File::open(p).map_err(|e| AppError::io(e, p))?;
        b.append_data(&mut h, name, Progress { inner: f, progress }).map_err(|e| io_or_cancel(e, p))?;
        file_done(it)?;
    }
    let mut out = b.into_inner().map_err(|e| AppError::io(e, target))?;
    out.flush().map_err(|e| AppError::io(e, target))?;
    drop(out);
    Ok(())
}
