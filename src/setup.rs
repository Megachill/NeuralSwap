//! Per-user install and uninstall, no administrator rights needed:
//! %LOCALAPPDATA%\Programs\NeuralSwap, Start-menu / desktop shortcuts and an
//! entry in Settings → Apps (HKCU uninstall key).

use crate::dist;
use std::fs;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SZ,
};

const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\NeuralSwap";

pub fn install_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    base.join("Programs").join("NeuralSwap")
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase().trim_end_matches('\\') == b.to_string_lossy().to_lowercase().trim_end_matches('\\')
}

/// True when this process runs from the per-user install folder.
pub fn running_installed() -> bool {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| same_path(p, &install_dir()))).unwrap_or(false)
}

/// A setup exe (one carrying a payload) started from outside the install
/// folder shows the installer instead of the app.
pub fn should_show_installer() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    !running_installed() && matches!(dist::appended_payload_offset(&exe), Ok(Some(_)))
}

/// The version of an existing installation, from its uninstall entry.
pub fn installed_version() -> Option<String> {
    crate::library::reg_string(HKEY_CURRENT_USER, UNINSTALL_KEY, "DisplayVersion")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_str(h: HKEY, name: &str, value: &str) {
    let n = wide(name);
    let v = wide(value);
    unsafe {
        RegSetValueExW(h, n.as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32);
    }
}

fn set_dword(h: HKEY, name: &str, value: u32) {
    let n = wide(name);
    unsafe {
        RegSetValueExW(h, n.as_ptr(), 0, REG_DWORD, &value as *const u32 as *const u8, 4);
    }
}

fn register(dir: &Path, size_kb: u32) {
    let key = wide(UNINSTALL_KEY);
    let mut h: HKEY = std::ptr::null_mut();
    let rc = unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, std::ptr::null(), &mut h, std::ptr::null_mut())
    };
    if rc != ERROR_SUCCESS {
        return;
    }
    let exe = dir.join("NeuralSwap.exe");
    set_str(h, "DisplayName", "NeuralSwap");
    set_str(h, "DisplayVersion", env!("CARGO_PKG_VERSION"));
    set_str(h, "Publisher", "craictechnology");
    set_str(h, "DisplayIcon", &exe.to_string_lossy());
    set_str(h, "InstallLocation", &dir.to_string_lossy());
    set_str(h, "UninstallString", &format!("\"{}\" --uninstall", exe.display()));
    set_dword(h, "EstimatedSize", size_kb);
    set_dword(h, "NoModify", 1);
    set_dword(h, "NoRepair", 1);
    unsafe { RegCloseKey(h) };
}

fn unregister() {
    let key = wide(UNINSTALL_KEY);
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
}

fn shortcut_paths() -> (PathBuf, PathBuf) {
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    (
        Path::new(&appdata).join("Microsoft\\Windows\\Start Menu\\Programs\\NeuralSwap.lnk"),
        Path::new(&profile).join("Desktop\\NeuralSwap.lnk"),
    )
}

fn powershell(script: &str) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .creation_flags(0x0800_0000)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn make_shortcut(lnk: &Path, target: &Path) -> bool {
    let q = |p: &Path| p.to_string_lossy().replace('\'', "''");
    powershell(&format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath='{}'; $s.WorkingDirectory='{}'; $s.IconLocation='{},0'; $s.Description='NeuralSwap - DLSS 5 neural rendering installer'; $s.Save()",
        q(lnk), q(target), q(target.parent().unwrap()), q(target)
    ))
}

/// Copies the app and extracts the payload carried by this setup exe.
/// `progress` gets (fraction 0..1, status text).
pub fn install(desktop: bool, progress: &mut dyn FnMut(f32, String)) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let target = dir.join("NeuralSwap.exe");
    progress(0.02, "Copying NeuralSwap.exe".into());
    // A running older copy keeps its image open; it can still be renamed.
    if target.exists() {
        let old = dir.join("NeuralSwap.exe.old");
        let _ = fs::remove_file(&old);
        fs::rename(&target, &old).map_err(|e| format!("Close NeuralSwap first ({e})"))?;
    }
    dist::write_app_only(&exe, &target).map_err(|e| e.to_string())?;

    // Fresh payload: extract beside, then swap, so a failure keeps the old one.
    // A public setup carries none: an existing payload is kept as it is.
    let payload = dir.join("payload");
    if dist::appended_entry_count(&exe) > 0 {
        let staging = dir.join("payload.new");
        let _ = fs::remove_dir_all(&staging);
        dist::extract(&exe, true, &staging, &mut |done, total| {
            progress(0.05 + 0.9 * done as f32 / total.max(1) as f32, format!("Extracting DLSS 5 files  {} / {} MB", done >> 20, total >> 20));
        })
        .map_err(|e| format!("Extracting the payload failed: {e}"))?;
        if payload.exists() {
            fs::remove_dir_all(&payload).map_err(|e| format!("Could not replace {}: {e}", payload.display()))?;
        }
        fs::rename(&staging, &payload).map_err(|e| e.to_string())?;
    }

    progress(0.97, "Creating shortcuts".into());
    let (start, desk) = shortcut_paths();
    make_shortcut(&start, &target);
    if desktop {
        make_shortcut(&desk, &target);
    }
    let size_kb = (dist::payload_size(&payload) / 1024) as u32;
    register(&dir, size_kb);
    progress(1.0, "Installed".into());
    Ok(target)
}

/// Removes shortcuts, the Apps entry and the install folder. Game backups
/// (`_DLSS5_Backup`) are never touched; settings optionally go too.
pub fn uninstall(remove_settings: bool) -> Result<(), String> {
    let (start, desk) = shortcut_paths();
    let _ = fs::remove_file(start);
    let _ = fs::remove_file(desk);
    unregister();
    let dir = install_dir();
    let _ = fs::remove_dir_all(dir.join("payload"));
    if remove_settings {
        let _ = fs::remove_dir_all(crate::state::data_dir());
    }
    // The running exe cannot delete itself: a hidden helper waits for this
    // process to exit (the user may keep the window open), then removes it.
    use std::os::windows::process::CommandExt;
    let script = format!(
        "Wait-Process -Id {} -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 500; Remove-Item -LiteralPath '{}' -Recurse -Force -ErrorAction SilentlyContinue",
        std::process::id(),
        dir.to_string_lossy().replace('\'', "''")
    );
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .creation_flags(0x0800_0000 | 0x0000_0200) // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// After a self-update, Settings → Apps should show the new version.
pub fn refresh_registration() {
    if running_installed() && installed_version().as_deref() != Some(env!("CARGO_PKG_VERSION")) {
        let dir = install_dir();
        register(&dir, (dist::payload_size(&dir.join("payload")) / 1024) as u32);
    }
}

/// Deletes the leftover of a previous self-update.
/// The previous process may still be closing, so retry for a while.
pub fn cleanup_old() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = PathBuf::from(format!("{}.old", exe.display()));
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..30 {
            if fs::remove_file(&old).is_ok() || !old.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}
