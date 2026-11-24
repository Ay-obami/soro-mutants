//! Ownership guard for every filesystem path Soro Mutants deletes.
//!
//! The `clean` command already documents the boundary: only scratch trees and
//! fixed managed cache names may be removed. Other cleanup paths — stale cache
//! invalidation, `--clear-cache` and runner scratch reuse — historically used
//! the same derived paths without that check. This module makes the boundary a
//! single, pure, deterministic predicate shared by all of them.
//!
//! "Owned" means: the path is a symlink-free child of the resolved managed root
//! and its first component is one of the fixed names Soro Mutants creates
//! itself. An externally supplied `--target-dir` is still allowed (it is a
//! documented feature), but only its managed children can be removed.

use std::{fmt, fs, path::Path};

/// Managed cache entry names directly under the resolved cache root.
pub const MANAGED_CACHE_NAMES: &[&str] = &[
    "baseline",
    "mutants-shared",
    "mutants-parallel",
    "progress.json",
    "cache-identity.json",
    "result-cache.json",
];

/// Name of the scratch worktree directly under the project root.
pub const SCRATCH_NAME: &str = ".soro-mutants-worktree";

/// Structured, actionable error returned when a deletion target is not owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupPathError {
    Symlink {
        path: std::path::PathBuf,
    },
    OutsideManagedRoot {
        path: std::path::PathBuf,
        root: std::path::PathBuf,
    },
    UnmanagedName {
        path: std::path::PathBuf,
        allowed: &'static [&'static str],
    },
    Metadata {
        path: std::path::PathBuf,
        reason: String,
    },
}

impl fmt::Display for CleanupPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Symlink { path } => {
                write!(
                    f,
                    "refusing to remove symlinked Soro Mutants path {}",
                    path.display()
                )
            }
            Self::OutsideManagedRoot { path, root } => write!(
                f,
                "refusing to remove path {} outside Soro Mutants root {}",
                path.display(),
                root.display()
            ),
            Self::UnmanagedName { path, allowed } => write!(
                f,
                "refusing to remove unmanaged path {}; managed names: {}",
                path.display(),
                allowed.join(", ")
            ),
            Self::Metadata { path, reason } => {
                write!(
                    f,
                    "cannot inspect Soro Mutants path {}: {}",
                    path.display(),
                    reason
                )
            }
        }
    }
}

impl std::error::Error for CleanupPathError {}

fn reject_symlink(path: &Path) -> Result<(), CleanupPathError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CleanupPathError::Symlink {
            path: path.to_path_buf(),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CleanupPathError::Metadata {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }),
    }
}

fn first_component(path: &Path, root: &Path) -> Result<std::path::PathBuf, CleanupPathError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| CleanupPathError::OutsideManagedRoot {
            path: path.to_path_buf(),
            root: root.to_path_buf(),
        })?;
    let component =
        relative
            .components()
            .next()
            .ok_or_else(|| CleanupPathError::OutsideManagedRoot {
                path: path.to_path_buf(),
                root: root.to_path_buf(),
            })?;
    Ok(component.as_os_str().to_owned().into())
}

/// Require that `path` is a managed cache child of `cache_root`.
pub fn require_owned_cache_path(path: &Path, cache_root: &Path) -> Result<(), CleanupPathError> {
    reject_symlink(cache_root)?;
    reject_symlink(path)?;
    let first = first_component(path, cache_root)?;
    let relative =
        path.strip_prefix(cache_root)
            .map_err(|_| CleanupPathError::OutsideManagedRoot {
                path: path.to_path_buf(),
                root: cache_root.to_path_buf(),
            })?;
    if relative.components().count() != 1
        || !MANAGED_CACHE_NAMES
            .iter()
            .any(|name| std::path::Path::new(name) == first.as_path())
    {
        return Err(CleanupPathError::UnmanagedName {
            path: path.to_path_buf(),
            allowed: MANAGED_CACHE_NAMES,
        });
    }
    Ok(())
}

/// Require that `path` lives inside a selected managed scratch root.
///
/// The scratch root itself and deeper worker directories are allowed. This is
/// used for both the historical project-local scratch directory and custom
/// `--scratch-dir` locations.
pub fn require_owned_scratch_location(
    path: &Path,
    scratch_root: &Path,
) -> Result<(), CleanupPathError> {
    reject_symlink(scratch_root)?;
    reject_symlink(path)?;
    if scratch_root.file_name().and_then(|name| name.to_str()) != Some(SCRATCH_NAME) {
        return Err(CleanupPathError::UnmanagedName {
            path: scratch_root.to_path_buf(),
            allowed: &[SCRATCH_NAME],
        });
    }
    if path != scratch_root && !path.starts_with(scratch_root) {
        return Err(CleanupPathError::OutsideManagedRoot {
            path: path.to_path_buf(),
            root: scratch_root.to_path_buf(),
        });
    }
    Ok(())
}

/// Require that `path` lives inside the project's default scratch worktree.
/// Kept as the project-root convenience wrapper used by clean and existing
/// callers.
pub fn require_owned_scratch_path(
    path: &Path,
    project_root: &Path,
) -> Result<(), CleanupPathError> {
    let scratch_root = project_root.join(SCRATCH_NAME);
    require_owned_scratch_location(path, &scratch_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "soro-pathguard-{name}-{}-{nanos}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[test]
    fn managed_cache_names_are_owned() {
        let root = temp_dir("cache");
        for name in MANAGED_CACHE_NAMES {
            let path = root.join(name);
            assert_eq!(require_owned_cache_path(&path, &root), Ok(()), "{name}");
        }
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn unmanaged_and_parent_paths_are_rejected() {
        let root = temp_dir("cache");
        let outside = root.parent().expect("parent").join("unmanaged-baseline");
        assert!(matches!(
            require_owned_cache_path(&outside, &root),
            Err(CleanupPathError::OutsideManagedRoot { .. })
        ));
        assert!(matches!(
            require_owned_cache_path(&root.join("notes.txt"), &root),
            Err(CleanupPathError::UnmanagedName { .. })
        ));
        assert!(matches!(
            require_owned_cache_path(&root.join("baseline").join("nested"), &root),
            Err(CleanupPathError::UnmanagedName { .. })
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn scratch_workers_are_owned_but_siblings_are_not() {
        let root = temp_dir("project");
        assert_eq!(
            require_owned_scratch_path(&root.join(SCRATCH_NAME).join("worker-0"), &root),
            Ok(())
        );
        assert!(matches!(
            require_owned_scratch_path(&root.join("src"), &root),
            Err(CleanupPathError::OutsideManagedRoot { .. })
                | Err(CleanupPathError::UnmanagedName { .. })
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn custom_scratch_root_owns_only_itself_and_worker_children() {
        let base = temp_dir("custom-scratch");
        let scratch = base.join(SCRATCH_NAME);
        assert_eq!(require_owned_scratch_location(&scratch, &scratch), Ok(()));
        assert_eq!(
            require_owned_scratch_location(&scratch.join("worker-2"), &scratch),
            Ok(())
        );
        assert!(matches!(
            require_owned_scratch_location(&base.join("other"), &scratch),
            Err(CleanupPathError::OutsideManagedRoot { .. })
        ));
        fs::remove_dir_all(base).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_targets_are_rejected_without_following_them() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("symlink");
        let external = temp_dir("external");
        fs::write(external.join("sentinel"), "keep").expect("sentinel");
        symlink(&external, root.join("baseline")).expect("symlink");
        assert_eq!(
            require_owned_cache_path(&root.join("baseline"), &root),
            Err(CleanupPathError::Symlink {
                path: root.join("baseline")
            })
        );
        assert_eq!(
            fs::read_to_string(external.join("sentinel")).expect("sentinel readable"),
            "keep"
        );
        fs::remove_dir_all(root).expect("cleanup");
        fs::remove_dir_all(external).expect("cleanup");
    }

    #[test]
    fn guard_is_pure_and_deterministic() {
        let root = temp_dir("pure");
        let path = root.join("baseline");
        let first = require_owned_cache_path(&path, &root);
        let second = require_owned_cache_path(&path, &root);
        assert_eq!(first, second);
        assert!(!path.exists(), "guard must not create filesystem entries");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
