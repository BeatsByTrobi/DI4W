//! Downloads and installs an update. Running programs can't overwrite their own files, so the last step runs
//! from a copy of this app in the temp folder (`--apply-update`): it waits for the island and this app to
//! exit and then either runs the new installer (for MSI installs) or copies the new files over the old ones,
//! with a backup to roll back to. Either way the island is started again as the signed-in user.

use di4w_core::updates::{ReleaseAsset, USER_AGENT, UpdateInfo, Version};
use di4w_core::{l, log};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Written by the installer (.msi) so updates go through the installer too.
const INSTALLER_KEY: &str = r"Software\DynamicIsland4Windows";

const ISLAND_EXE: &str = "DynamicIsland.exe";
const SETTINGS_EXE: &str = "DynamicIsland.Settings.exe";

pub fn work_folder() -> PathBuf {
    std::env::temp_dir().join("DI4W-update")
}

/// The folder that holds DynamicIsland.exe, with this app in its Settings subfolder (the published layout).
/// `None` when running from a build folder, where updating would overwrite build output.
pub fn install_folder() -> Option<PathBuf> {
    let here = crate::system::exe_folder()?;
    let parent = here.parent()?.to_path_buf();
    let in_settings = here.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Settings"));
    (in_settings && parent.join(ISLAND_EXE).is_file()).then_some(parent)
}

/// True when this copy was installed with the .msi installer (it records its folder in the registry).
pub fn installed_with_msi() -> bool {
    let (Some(recorded), Some(here)) = (di4w_core::registry::read_string(INSTALLER_KEY, "InstallFolder"), install_folder()) else {
        return false;
    };
    let normalize = |p: &str| p.trim_end_matches('\\').to_lowercase();
    normalize(&recorded) == normalize(&here.display().to_string())
}

/// "installer", "zip" or "from source", for problem reports.
pub fn install_type() -> &'static str {
    if installed_with_msi() {
        "installer"
    } else if install_folder().is_some() {
        "zip"
    } else {
        "from source"
    }
}

/// The installer for MSI installs (when the release has one), otherwise the zip.
pub fn pick_asset(update: &UpdateInfo) -> Option<&ReleaseAsset> {
    if installed_with_msi() && update.msi.is_some() {
        return update.msi.as_ref();
    }
    update.zip.as_ref().or(update.msi.as_ref())
}

/// Downloads and verifies the update. Returns the .msi, or the folder with the unpacked new files.
pub fn download(update: &UpdateInfo, progress: &dyn Fn(f64), cancel: &AtomicBool) -> Result<PathBuf, String> {
    let asset = pick_asset(update).ok_or_else(|| l::t("This release has no Windows package.").to_string())?;
    let folder = work_folder().join(update.version.to_string());
    if folder.exists() {
        std::fs::remove_dir_all(&folder).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

    let file = folder.join(&asset.name);
    download_file(asset, &file, progress, cancel)?;
    verify(&file, asset)?;
    if asset.name.to_lowercase().ends_with(".msi") {
        return Ok(file);
    }
    unpack(&file, &folder.join("files"))
}

fn download_file(asset: &ReleaseAsset, target: &Path, progress: &dyn Fn(f64), cancel: &AtomicBool) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(30))).build().into();
    let mut response = agent.get(&asset.url).header("User-Agent", USER_AGENT).call().map_err(|e| e.to_string())?;
    let mut source = response.body_mut().as_reader();
    let mut output = std::fs::File::create(target).map_err(|e| e.to_string())?;
    let mut buffer = vec![0u8; 81920];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let read = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
        done += read as u64;
        progress(if asset.size > 0 { done as f64 / asset.size as f64 } else { 0.0 });
    }
    output.flush().map_err(|e| e.to_string())
}

/// The downloaded file has the release's size and, when GitHub reports one, its SHA-256.
pub fn verify(file: &Path, asset: &ReleaseAsset) -> Result<(), String> {
    let length = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    if length != asset.size {
        return Err(l::t("The download is incomplete. Please try again.").into());
    }
    if let Some(expected) = &asset.sha256 {
        let actual = sha256_hex(file).map_err(|e| e.to_string())?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(l::t("The download doesn't match the release (checksum mismatch), so it wasn't installed.").into());
        }
    }
    Ok(())
}

pub fn sha256_hex(file: &Path) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut input = std::fs::File::open(file)?;
    std::io::copy(&mut input, &mut hasher)?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02X}")).collect())
}

/// Unpacks the zip (entries that would land outside `into` are refused) and returns the folder with
/// DynamicIsland.exe and Settings\DynamicIsland.Settings.exe.
pub fn unpack(zip_file: &Path, into: &Path) -> Result<PathBuf, String> {
    let file = std::fs::File::open(zip_file).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    archive.extract(into).map_err(|e| e.to_string())?;
    let island = find_file(into, ISLAND_EXE).ok_or_else(|| l::t("The update package doesn't contain DynamicIsland.exe.").to_string())?;
    let root = island.parent().unwrap_or(into).to_path_buf();
    if !root.join("Settings").join(SETTINGS_EXE).is_file() {
        return Err(l::t("The update package doesn't contain the settings app.").into());
    }
    Ok(root)
}

fn find_file(folder: &Path, name: &str) -> Option<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(folder).ok()?.flatten().map(|e| e.path()).collect();
    entries.sort();
    if let Some(found) = entries.iter().find(|p| p.is_file() && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name))) {
        return Some(found.clone());
    }
    entries.iter().filter(|p| p.is_dir()).find_map(|dir| find_file(dir, name))
}

/// Why the update didn't start.
#[derive(Debug)]
pub enum InstallError {
    /// The user said no to the administrator prompt.
    Declined,
    Other(String),
}

/// Starts the install step from a copy of this app, asks the island to quit, and returns; the caller then
/// closes this app. `payload`: the downloaded .msi, or the folder with the new files.
pub fn install(payload: &Path, install_folder: &Path, version: Version) -> Result<(), InstallError> {
    let helper_folder = work_folder().join("installer");
    let _ = std::fs::remove_dir_all(&helper_folder);
    std::fs::create_dir_all(&helper_folder).map_err(|e| InstallError::Other(e.to_string()))?;
    let helper = helper_folder.join(SETTINGS_EXE);
    let me = std::env::current_exe().map_err(|e| InstallError::Other(e.to_string()))?;
    std::fs::copy(&me, &helper).map_err(|e| InstallError::Other(e.to_string()))?;

    let log_file = di4w_core::paths::log_folder().join("update.log");
    let args = [
        "--apply-update".to_string(),
        "--payload".into(),
        payload.display().to_string(),
        "--target".into(),
        install_folder.display().to_string().trim_end_matches('\\').to_string(),
        "--version".into(),
        version.to_string(),
        "--log".into(),
        log_file.display().to_string(),
    ];
    let is_msi = payload.extension().is_some_and(|e| e.eq_ignore_ascii_case("msi"));
    // Copying into a protected folder (e.g. Program Files) needs admin rights; ask only then.
    // The installer is per-user and needs none.
    if !is_msi && !can_write(install_folder) {
        run_elevated(&helper, &args)?;
    } else {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new(&helper)
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| InstallError::Other(e.to_string()))?;
    }
    log::info(format!("Installing update {version} from {} into {}", payload.display(), install_folder.display()));
    crate::system::signal_island_quit();
    Ok(())
}

fn can_write(folder: &Path) -> bool {
    let probe = folder.join(format!(".write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// `"a b"` quoting for a command line (none of our arguments contain quotes or end in a backslash).
fn quote(arg: &str) -> String {
    format!("\"{arg}\"")
}

fn run_elevated(exe: &Path, args: &[String]) -> Result<(), InstallError> {
    use di4w_core::wide::wide;
    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    use windows::core::{PCWSTR, w};
    let file = wide(&exe.display().to_string());
    let parameters = wide(&args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
        if e.code() == ERROR_CANCELLED.to_hresult() { InstallError::Declined } else { InstallError::Other(e.message()) }
    })
}

// ───────────────────────────── the install step (runs from the temp copy) ─────────────────────────────

/// `--apply-update --payload <file or folder> --target <install folder> --version <x.y.z> --log <file>`.
pub fn apply_update(args: &[String]) -> i32 {
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    let (payload, target, version, log_file) =
        (PathBuf::from(value("--payload")), PathBuf::from(value("--target")), value("--version"), PathBuf::from(value("--log")));
    let write_log = |message: &str| {
        if let Some(dir) = log_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_file) {
            let _ = writeln!(f, "{} {message}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        }
    };
    let island = target.join(ISLAND_EXE);

    // Wait for this installation's island and settings app to close (they were asked to), then make sure.
    wait_for_apps_to_exit(&target, Duration::from_secs(20));
    std::thread::sleep(Duration::from_millis(500));

    if payload.extension().is_some_and(|e| e.eq_ignore_ascii_case("msi")) {
        // Installed with the installer: let it upgrade, so Windows' "Installed apps" stays correct.
        let msi_log = log_file.with_file_name("update-msi.log");
        let status = std::process::Command::new("msiexec.exe")
            .arg("/i")
            .arg(&payload)
            .args(["/qb-", "/norestart", "/l*v"])
            .arg(&msi_log)
            .status();
        match status.map(|s| s.code().unwrap_or(-1)) {
            Ok(0) | Ok(3010) => write_log(&format!("Updated to {version} with the installer")),
            Ok(code) => write_log(&format!(
                "Installer failed with code {code}; see {}. The previous version is still installed.",
                msi_log.display()
            )),
            Err(e) => write_log(&format!("Could not start the installer: {e}. The previous version is still installed.")),
        }
        start_island_as_user(&island);
        return 0;
    }

    // Copy-over update with a backup, so a failed copy never leaves a half-updated mix of files.
    let backup = payload.parent().unwrap_or(&payload).join("backup");
    match replace_files(&payload, &target, &backup, &copy_with_retries) {
        Ok(()) => write_log(&format!("Updated {} to {version}", target.display())),
        Err(Failure::Backup(e)) => write_log(&format!("Update to {version} cancelled: couldn't back up {} ({e})", target.display())),
        Err(Failure::Copy(e)) => write_log(&format!("Update to {version} failed ({e}); restored the previous version")),
    }
    start_island_as_user(&island);
    0
}

/// Start the island as the signed-in user, never as administrator (even if this step was elevated).
fn start_island_as_user(island: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(island).spawn();
}

/// The island and settings app running from `target`, apart from this process.
fn apps_running_from(target: &Path) -> Vec<u32> {
    let prefix = format!("{}\\", target.display().to_string().trim_end_matches('\\').to_lowercase());
    let own = std::process::id();
    crate::system::processes()
        .into_iter()
        .filter(|p| p.id != own && (p.name.eq_ignore_ascii_case(ISLAND_EXE) || p.name.eq_ignore_ascii_case(SETTINGS_EXE)))
        .filter(|p| crate::system::process_path(p.id).is_some_and(|path| path.display().to_string().to_lowercase().starts_with(&prefix)))
        .map(|p| p.id)
        .collect()
}

fn wait_for_apps_to_exit(target: &Path, timeout: Duration) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    let deadline = Instant::now() + timeout;
    while !apps_running_from(target).is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(300));
    }
    for id in apps_running_from(target) {
        unsafe {
            if let Ok(process) = OpenProcess(PROCESS_TERMINATE, false, id) {
                let _ = TerminateProcess(process, 1);
                let _ = CloseHandle(process);
            }
        }
    }
}

/// Where replacing the files went wrong.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// Nothing was changed: the current files couldn't be backed up.
    Backup(String),
    /// Copying or checking the new files failed; the backup was put back.
    Copy(String),
}

/// Copies `payload` over `target`: backs `target` up first, checks every new file arrived intact, and puts the
/// backup back if anything went wrong. Files only in `target` stay.
pub fn replace_files(payload: &Path, target: &Path, backup: &Path, copy: &dyn Fn(&Path, &Path) -> std::io::Result<()>) -> Result<(), Failure> {
    let _ = std::fs::remove_dir_all(backup);
    copy_tree(target, backup, &|from, to| std::fs::copy(from, to).map(|_| ())).map_err(|e| Failure::Backup(e.to_string()))?;

    let result = copy_tree(payload, target, copy).map_err(|e| e.to_string()).and_then(|()| check_copied(payload, target));
    if let Err(e) = result {
        let _ = copy_tree(backup, target, &copy_with_retries);
        let _ = std::fs::remove_dir_all(backup);
        return Err(Failure::Copy(e));
    }
    let _ = std::fs::remove_dir_all(backup);
    Ok(())
}

fn copy_tree(from: &Path, to: &Path, copy: &dyn Fn(&Path, &Path) -> std::io::Result<()>) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target, copy)?;
        } else {
            copy(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Every file of `payload` is in `target` with the same contents.
fn check_copied(payload: &Path, target: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(payload).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let other = target.join(entry.file_name());
        if entry.path().is_dir() {
            check_copied(&entry.path(), &other)?;
        } else if sha256_hex(&entry.path()).ok() != sha256_hex(&other).ok() {
            return Err(format!("{} didn't copy", other.display()));
        }
    }
    Ok(())
}

/// A file that's still locked (the island closing) gets a few more tries.
fn copy_with_retries(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::copy(from, to) {
            Ok(_) => return Ok(()),
            Err(_) if attempt < 10 => {
                attempt += 1;
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Release notes are Markdown; show them as readable plain text (the full page is one click away).
pub fn plain_text(markdown: &str) -> String {
    let link = regex::Regex::new(r"\[([^\]]+)\]\([^)]*\)").unwrap();
    let heading = regex::Regex::new(r"(?m)^\s{0,3}#{1,6}\s*").unwrap();
    let emphasis = regex::Regex::new(r"(\*\*|__|`)").unwrap();
    let bullet = regex::Regex::new(r"(?m)^\s*[-*]\s+").unwrap();
    let text = link.replace_all(markdown, "$1");
    let text = heading.replace_all(&text, "");
    let text = emphasis.replace_all(&text, "");
    let text = bullet.replace_all(&text, "• ");
    text.replace("\r\n", "\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    /// An installed copy (old version) and the unpacked update (new version).
    fn layout() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("Dynamic Island");
        let payload = root.path().join("update").join("files");
        let backup = root.path().join("update").join("backup");
        write(&target.join(ISLAND_EXE), "island 1");
        write(&target.join("Settings").join(SETTINGS_EXE), "settings 1");
        write(&target.join("notes.txt"), "the user's own file");
        write(&payload.join(ISLAND_EXE), "island 2");
        write(&payload.join("Settings").join(SETTINGS_EXE), "settings 2");
        write(&payload.join("tools").join("island.ps1"), "script 2");
        (root, target, payload, backup)
    }

    #[test]
    fn replaces_the_files() {
        let (_root, target, payload, backup) = layout();
        let copy = |from: &Path, to: &Path| std::fs::copy(from, to).map(|_| ());
        assert_eq!(replace_files(&payload, &target, &backup, &copy), Ok(()));
        assert_eq!(read(&target.join(ISLAND_EXE)), "island 2");
        assert_eq!(read(&target.join("Settings").join(SETTINGS_EXE)), "settings 2");
        assert_eq!(read(&target.join("tools").join("island.ps1")), "script 2");
        assert_eq!(read(&target.join("notes.txt")), "the user's own file");
        assert!(!backup.exists());
    }

    #[test]
    fn rolls_back_when_a_file_cannot_be_copied() {
        let (_root, target, payload, backup) = layout();
        // The settings app's exe is still locked: copying it fails after the island was already replaced.
        let copy = |from: &Path, to: &Path| {
            if from.ends_with(Path::new("Settings").join(SETTINGS_EXE)) {
                Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "in use"))
            } else {
                std::fs::copy(from, to).map(|_| ())
            }
        };
        let result = replace_files(&payload, &target, &backup, &copy);
        assert!(matches!(result, Err(Failure::Copy(_))), "{result:?}");
        assert_eq!(read(&target.join(ISLAND_EXE)), "island 1");
        assert_eq!(read(&target.join("Settings").join(SETTINGS_EXE)), "settings 1");
        assert!(!backup.exists());
    }

    #[test]
    fn rolls_back_when_a_copy_is_damaged() {
        let (_root, target, payload, backup) = layout();
        let copy = |from: &Path, to: &Path| {
            std::fs::copy(from, to)?;
            if from.ends_with(ISLAND_EXE) {
                std::fs::write(to, "half a file")?;
            }
            Ok(())
        };
        assert!(matches!(replace_files(&payload, &target, &backup, &copy), Err(Failure::Copy(_))));
        assert_eq!(read(&target.join(ISLAND_EXE)), "island 1");
    }

    #[test]
    fn checks_size_and_checksum() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("DI4W-v9.0.0-win-x64.zip");
        std::fs::write(&file, b"package").unwrap();
        let sha = sha256_hex(&file).unwrap();
        let asset = |size, sha256: Option<&str>| ReleaseAsset { name: "x".into(), url: String::new(), size, sha256: sha256.map(Into::into) };
        assert!(verify(&file, &asset(7, Some(&sha.to_lowercase()))).is_ok());
        assert!(verify(&file, &asset(7, None)).is_ok());
        assert!(verify(&file, &asset(8, None)).is_err());
        assert!(verify(&file, &asset(7, Some("00"))).is_err());
    }

    #[test]
    fn unpacks_a_release_zip() {
        let folder = tempfile::tempdir().unwrap();
        let zip_path = folder.path().join("DI4W-v9.0.0-win-x64.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("DI4W/DynamicIsland.exe", options).unwrap();
            zip.write_all(b"island").unwrap();
            zip.start_file("DI4W/Settings/DynamicIsland.Settings.exe", options).unwrap();
            zip.write_all(b"settings").unwrap();
            zip.finish().unwrap();
        }
        let root = unpack(&zip_path, &folder.path().join("files")).unwrap();
        assert!(root.ends_with("DI4W"));
        assert_eq!(read(&root.join(ISLAND_EXE)), "island");
    }

    #[test]
    fn release_notes_as_plain_text() {
        let notes = "## What's new\n- **Faster** start\n* See [the docs](https://x.y) and `code`";
        assert_eq!(plain_text(notes), "What's new\n• Faster start\n• See the docs and code");
    }
}
