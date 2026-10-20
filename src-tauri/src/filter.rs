//! Total Commander style name masks: `*.jpg;*.png | thumbs*`
//! – patterns separated by `;` (or `,`), exclusions after `|`,
//! a pattern without wildcards matches as a substring (`abc` == `*abc*`).

use crate::error::{AppError, AppResult};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

#[derive(Clone)]
pub struct NameFilter {
    include: Option<GlobSet>,
    exclude: Option<GlobSet>,
}

fn build(patterns: &str, case_sensitive: bool) -> AppResult<Option<GlobSet>> {
    let mut b = GlobSetBuilder::new();
    let mut any = false;
    for raw in patterns.split([';', ',']) {
        let p = raw.trim();
        if p.is_empty() || p == "*" || p == "*.*" {
            continue;
        }
        let pat = if p.contains(['*', '?', '[']) { p.to_string() } else { format!("*{p}*") };
        let glob = GlobBuilder::new(&pat)
            .case_insensitive(!case_sensitive)
            .literal_separator(false)
            .backslash_escape(false)
            .build()
            .map_err(|e| AppError::invalid(format!("Invalid mask '{p}': {e}")))?;
        b.add(glob);
        any = true;
    }
    if !any {
        return Ok(None);
    }
    b.build().map(Some).map_err(|e| AppError::invalid(e.to_string()))
}

impl NameFilter {
    pub fn parse(mask: &str, case_sensitive: bool) -> AppResult<Self> {
        let (inc, exc) = match mask.split_once('|') {
            Some((a, b)) => (a, b),
            None => (mask, ""),
        };
        Ok(Self { include: build(inc, case_sensitive)?, exclude: build(exc, case_sensitive)? })
    }

    pub fn is_noop(&self) -> bool {
        self.include.is_none() && self.exclude.is_none()
    }

    pub fn matches(&self, name: &str) -> bool {
        if let Some(inc) = &self.include {
            if !inc.is_match(name) {
                return false;
            }
        }
        if let Some(exc) = &self.exclude {
            if exc.is_match(name) {
                return false;
            }
        }
        true
    }

    /// Exclusion check only – used for directories while traversing.
    pub fn excluded(&self, name: &str) -> bool {
        self.exclude.as_ref().map(|e| e.is_match(name)).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks() {
        let f = NameFilter::parse("*.jpg;*.PNG | thumb*", false).unwrap();
        assert!(f.matches("a.JPG"));
        assert!(f.matches("b.png"));
        assert!(!f.matches("thumb.jpg"));
        assert!(!f.matches("c.gif"));
        let f = NameFilter::parse("report", false).unwrap();
        assert!(f.matches("Monthly-Report-2024.pdf"));
        assert!(NameFilter::parse("*.*", false).unwrap().is_noop());
        let f = NameFilter::parse("| node_modules;.git", false).unwrap();
        assert!(f.excluded("node_modules"));
        assert!(f.matches("index.js"));
    }
}
