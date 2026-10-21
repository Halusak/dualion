//! Multi-rename backend: renames many files in one folder atomically enough
//! to handle swaps (a→b, b→a) and case-only changes.

use crate::error::{AppError, AppResult, ErrorCode};
use crate::local;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RenamePair {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameOutcome {
    pub from: String,
    pub to: String,
    pub error: Option<AppError>,
}

fn fold(name: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        name.to_lowercase()
    } else {
        name.to_string()
    }
}

pub fn batch_rename(dir: &str, pairs: Vec<RenamePair>) -> AppResult<Vec<RenameOutcome>> {
    let base = local::normalize(dir);
    let sources: HashSet<String> = pairs.iter().filter(|p| p.from != p.to).map(|p| fold(&p.from)).collect();
    let mut seen_targets: HashSet<String> = HashSet::new();
    let mut out: Vec<RenameOutcome> = Vec::with_capacity(pairs.len());
    let mut valid: Vec<usize> = Vec::new();

    for p in &pairs {
        let mut err = None;
        if p.from == p.to {
            out.push(RenameOutcome { from: p.from.clone(), to: p.to.clone(), error: None });
            continue;
        }
        if let Err(e) = local::check_name(&p.to) {
            err = Some(e);
        } else if !seen_targets.insert(fold(&p.to)) {
            err = Some(AppError::new(ErrorCode::AlreadyExists, "Duplicate target name"));
        } else if base.join(&p.to).exists() && !sources.contains(&fold(&p.to)) {
            err = Some(AppError::new(ErrorCode::AlreadyExists, "Target already exists").with_path(base.join(&p.to)));
        }
        if err.is_none() {
            valid.push(out.len());
        }
        out.push(RenameOutcome { from: p.from.clone(), to: p.to.clone(), error: err });
    }

    // Phase 1: move every source out of the way.
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let mut staged: Vec<(usize, std::path::PathBuf)> = Vec::new();
    for &i in &valid {
        let src = base.join(&out[i].from);
        let tmp = base.join(format!(".dualion-{tag}-{i}"));
        match fs::rename(&src, &tmp) {
            Ok(()) => staged.push((i, tmp)),
            Err(e) => out[i].error = Some(AppError::io(e, &src)),
        }
    }
    // Phase 2: give them their final names (restore on failure).
    for (i, tmp) in staged {
        let dst = base.join(&out[i].to);
        // A source that was meant to make room may have stayed (its own new
        // name was rejected): never replace it, rename() would do so silently.
        if fs::symlink_metadata(&dst).is_ok() {
            let _ = fs::rename(&tmp, base.join(&out[i].from));
            out[i].error = Some(AppError::new(ErrorCode::AlreadyExists, "Target already exists").with_path(&dst));
            continue;
        }
        if let Err(e) = fs::rename(&tmp, &dst) {
            let _ = fs::rename(&tmp, base.join(&out[i].from));
            out[i].error = Some(AppError::io(e, &dst));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_and_conflicts() {
        let dir = std::env::temp_dir().join(format!("dualion-ren-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "A").unwrap();
        fs::write(dir.join("b.txt"), "B").unwrap();
        fs::write(dir.join("c.txt"), "C").unwrap();
        let res = batch_rename(
            &dir.to_string_lossy(),
            vec![
                RenamePair { from: "a.txt".into(), to: "b.txt".into() },
                RenamePair { from: "b.txt".into(), to: "a.txt".into() },
                RenamePair { from: "c.txt".into(), to: "a.txt".into() },
            ],
        )
        .unwrap();
        assert!(res[0].error.is_none() && res[1].error.is_none());
        assert!(res[2].error.is_some());
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "B");
        assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "A");
        assert_eq!(fs::read_to_string(dir.join("c.txt")).unwrap(), "C");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// a→b may replace b only when b really moved away; here b's own new
    /// name is rejected, so b stays and must not be overwritten.
    #[test]
    fn never_overwrites_a_source_that_stayed() {
        let dir = std::env::temp_dir().join(format!("dualion-ren-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "A").unwrap();
        fs::write(dir.join("b.txt"), "B").unwrap();
        let res = batch_rename(
            &dir.to_string_lossy(),
            vec![RenamePair { from: "a.txt".into(), to: "b.txt".into() }, RenamePair { from: "b.txt".into(), to: "bad/name".into() }],
        )
        .unwrap();
        assert!(res[0].error.is_some() && res[1].error.is_some());
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "A");
        assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "B");
        fs::remove_dir_all(&dir).unwrap();
    }
}
