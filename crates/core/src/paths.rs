//! Where both apps keep their files: `%AppData%\DynamicIsland` (settings, status, crash report, logs).

use std::path::PathBuf;
use std::sync::RwLock;

static OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// `%AppData%\DynamicIsland`, or the folder tests set with [`set_data_folder`].
pub fn data_folder() -> PathBuf {
    if let Some(folder) = OVERRIDE.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return folder;
    }
    let app_data = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    app_data.join("DynamicIsland")
}

/// Points every file in this crate at another folder (tests use a temporary one); `None` goes back to AppData.
pub fn set_data_folder(folder: Option<PathBuf>) {
    *OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = folder;
}

pub fn settings_file() -> PathBuf {
    data_folder().join("settings.json")
}

pub fn status_file() -> PathBuf {
    data_folder().join("status.json")
}

pub fn crash_file() -> PathBuf {
    data_folder().join("crash.txt")
}

pub fn log_folder() -> PathBuf {
    data_folder().join("logs")
}

/// Writes `contents` next to `path` and swaps it in, so the other app never reads a half-written file.
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    std::fs::write(&temp, contents)?;
    // On Windows std::fs::rename replaces an existing file (MoveFileExW with MOVEFILE_REPLACE_EXISTING).
    std::fs::rename(&temp, path)
}
