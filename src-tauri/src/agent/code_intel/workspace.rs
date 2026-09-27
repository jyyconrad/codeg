use std::path::{Path, PathBuf};

pub fn canonical_workspace(path: &Path) -> PathBuf {
    if let Ok(path) = std::fs::canonicalize(path) {
        return path;
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_workspace_collapses_dot_alias() {
        let dir = tempfile::tempdir().unwrap();
        let direct = canonical_workspace(dir.path());
        let alias = canonical_workspace(&dir.path().join("."));
        assert_eq!(direct, alias);
        assert!(direct.is_absolute());
    }

    #[test]
    fn canonical_workspace_keeps_missing_absolute_path() {
        let missing =
            std::env::temp_dir().join(format!("codeg-missing-workspace-{}", std::process::id()));
        assert!(!missing.exists());
        assert_eq!(canonical_workspace(&missing), missing);
    }

    #[test]
    fn canonical_workspace_joins_relative_path() {
        let relative = Path::new("definitely-not-a-codeg-workspace");
        let got = canonical_workspace(relative);
        assert!(got.is_absolute());
        assert!(got.ends_with(relative));
    }
}
