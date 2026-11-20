//! Owns one test's app data, worktree, support files and separate engine homes.

use std::{fs, io, path::Path};

use tempfile::TempDir;

/// Engine whose otherwise user-global configuration must be isolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// Official pi CLI.
    Pi,
    /// Official OMP CLI.
    Omp,
    /// Official DSH CLI.
    Dsh,
}

impl Engine {
    /// The real adapter's configuration-directory variable (OMP also uses pi's variable).
    pub fn config_var(self) -> &'static str {
        match self {
            Self::Pi | Self::Omp => "PI_CODING_AGENT_DIR",
            Self::Dsh => "DSH_HOME",
        }
    }

    fn directory(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Omp => "omp",
            Self::Dsh => "dsh",
        }
    }
}

/// An RAII test sandbox. Drop sessions and child processes before dropping this owner.
#[derive(Debug)]
pub struct Isolation {
    root: TempDir,
}

impl Isolation {
    /// Creates fresh, canonical directories without reading or mutating the user's environment.
    ///
    /// # Errors
    /// Returns filesystem errors creating or canonicalizing the sandbox.
    pub fn new() -> io::Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("autocraft-smoke-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir())?)?;
        for name in ["data", "workspace", "support", "pi", "omp", "dsh"] {
            fs::create_dir(root.path().join(name))?;
        }
        Ok(Self { root })
    }

    /// Root owned and removed by this sandbox.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// App data directory, passed only to the app's `--data-dir` argument.
    pub fn data_dir(&self) -> std::path::PathBuf {
        self.root.path().join("data")
    }

    /// Disposable cwd, never a developer worktree.
    pub fn cwd(&self) -> std::path::PathBuf {
        self.root.path().join("workspace")
    }

    /// Generated per-session extensions and other support files.
    pub fn support_dir(&self) -> std::path::PathBuf {
        self.root.path().join("support")
    }

    /// Independent engine config root, passed as `StartSession.config_root` or a child env var.
    pub fn config_dir(&self, engine: Engine) -> std::path::PathBuf {
        self.root.path().join(engine.directory())
    }

    /// Removes the sandbox explicitly, exposing cleanup failures to the caller.
    ///
    /// # Errors
    /// Returns an error if files remain locked or cannot be removed.
    pub fn close(self) -> io::Result<()> {
        self.root.close()
    }
}
