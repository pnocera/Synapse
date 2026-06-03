//! Single-instance guard for the Synapse daemon (`--mode http`).
//!
//! Guarantees that at most one daemon process owns a given RocksDB directory at
//! a time. The guard is acquired at startup **before** RocksDB is opened, so a
//! duplicate launch fails fast with a clear, actionable error that names the
//! current holder PID — instead of surfacing later as a cryptic RocksDB `LOCK`
//! failure deep inside a tool call (the exact symptom that motivated this work).
//!
//! Mechanism: an OS advisory exclusive file lock (`fs2`) on `<db>/daemon.lock`.
//! Chosen over a bare Win32 named mutex because the lock is released
//! automatically by the OS when the holding process dies, so a crashed daemon
//! never wedges future launches, and because it is cross-platform (so the
//! behavior is testable off-Windows).
//!
//! The holder PID is deliberately stored in a **separate** `<db>/daemon.pid`
//! file rather than inside the lock file. On Windows `fs2` uses `LockFileEx`,
//! whose exclusive lock is a *mandatory whole-file* lock: while held, no other
//! process can even read the locked file. Storing the PID in an unlocked
//! sidecar keeps it readable by duplicate launchers and by
//! `synapse-mcp doctor`, while the lock file itself stays empty.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process,
};

use fs2::FileExt;

/// Empty file created inside the RocksDB directory used purely as the daemon
/// single-instance advisory lock token.
pub const DAEMON_LOCK_FILE: &str = "daemon.lock";

/// Unlocked sidecar file holding the current lock holder's PID (diagnostics).
pub const DAEMON_PID_FILE: &str = "daemon.pid";

/// Failure modes when acquiring the daemon single-instance lock.
#[derive(Debug)]
pub enum SingleInstanceError {
    /// Another daemon already holds the lock for this DB path.
    AlreadyRunning {
        lock_path: PathBuf,
        holder_pid: Option<u32>,
    },
    /// The lock file could not be created or locked for a reason other than an
    /// existing holder.
    Io { lock_path: PathBuf, detail: String },
}

impl std::fmt::Display for SingleInstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning {
                lock_path,
                holder_pid,
            } => write!(
                f,
                "another synapse-mcp daemon already owns {} (holder pid {}); stop the other daemon before starting a second one, or point this daemon at a different --db path",
                lock_path.display(),
                holder_pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string()),
            ),
            Self::Io { lock_path, detail } => write!(
                f,
                "failed to acquire daemon single-instance lock {}: {detail}",
                lock_path.display(),
            ),
        }
    }
}

impl std::error::Error for SingleInstanceError {}

/// Holds the daemon single-instance advisory file lock for the lifetime of the
/// process. Dropping the guard releases the lock and removes the PID sidecar
/// (the OS also releases the lock automatically if the process dies).
#[must_use = "dropping the guard immediately releases the single-instance lock"]
pub struct SingleInstanceGuard {
    file: File,
    lock_path: PathBuf,
    pid_path: PathBuf,
}

impl SingleInstanceGuard {
    /// Acquire the single-instance lock for `db_path`.
    ///
    /// # Errors
    ///
    /// Returns [`SingleInstanceError::AlreadyRunning`] (naming the current
    /// holder PID when readable) if another daemon already owns the lock, or
    /// [`SingleInstanceError::Io`] if the lock file cannot be created/locked.
    pub fn acquire(db_path: &Path) -> Result<Self, SingleInstanceError> {
        let lock_path = db_path.join(DAEMON_LOCK_FILE);
        let pid_path = db_path.join(DAEMON_PID_FILE);
        fs::create_dir_all(db_path).map_err(|err| SingleInstanceError::Io {
            lock_path: lock_path.clone(),
            detail: format!("create db directory: {err}"),
        })?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|err| SingleInstanceError::Io {
                lock_path: lock_path.clone(),
                detail: format!("open lock file: {err}"),
            })?;

        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                write_pid_file(&pid_path, process::id()).map_err(|err| {
                    SingleInstanceError::Io {
                        lock_path: lock_path.clone(),
                        detail: format!("record holder pid: {err}"),
                    }
                })?;
                Ok(Self {
                    file,
                    lock_path,
                    pid_path,
                })
            }
            Err(_) => Err(SingleInstanceError::AlreadyRunning {
                holder_pid: read_pid_file(&pid_path),
                lock_path,
            }),
        }
    }

    /// Path of the lock file backing this guard.
    #[must_use]
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    /// Read the PID recorded for `db_path`'s daemon, if any. Used by diagnostics
    /// (`doctor`); does not imply the holder is still alive.
    #[must_use]
    pub fn recorded_holder_pid(db_path: &Path) -> Option<u32> {
        read_pid_file(&db_path.join(DAEMON_PID_FILE))
    }
}

fn write_pid_file(pid_path: &Path, pid: u32) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(pid_path)?;
    file.write_all(pid.to_string().as_bytes())?;
    file.flush()
}

fn read_pid_file(pid_path: &Path) -> Option<u32> {
    fs::read_to_string(pid_path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
        let _ = fs::remove_file(&self.pid_path);
    }
}
