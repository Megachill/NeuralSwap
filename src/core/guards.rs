//! Read-only preflight checks: is the game running, which GPU is this.
//! Nothing here disables, bypasses or alters anti-cheat.

use super::{fail, Result};
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};

pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

pub fn processes() -> Vec<ProcessInfo> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return out;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok {
            let pid = entry.th32ProcessID;
            let name = wide_to_string(&entry.szExeFile);
            let mut path = None;
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if !h.is_null() {
                let mut buf = [0u16; 1024];
                let mut len = buf.len() as u32;
                if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0 {
                    path = Some(String::from_utf16_lossy(&buf[..len as usize]));
                }
                CloseHandle(h);
            }
            out.push(ProcessInfo { pid, name, path });
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// Windows keeps a running executable's image open, so failing to open it
/// for writing with a sharing violation means it is running.
fn executable_locked(exe: &Path) -> bool {
    match std::fs::OpenOptions::new().write(true).open(exe) {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(32),
    }
}

/// Processes whose image lives inside the game folder.
pub fn running_in(game_dir: &Path, exe: Option<&Path>) -> Vec<String> {
    let root = format!("{}\\", game_dir.to_string_lossy().trim_end_matches('\\').to_lowercase());
    let exe_name = exe.and_then(|e| e.file_name()).map(|n| n.to_string_lossy().to_lowercase());
    let me = std::process::id();
    processes()
        .into_iter()
        .filter(|p| p.pid != me)
        .filter(|p| match &p.path {
            Some(path) => path.to_lowercase().starts_with(&root),
            // Protected processes may hide their path: match by name only.
            None => exe_name.as_deref() == Some(p.name.to_lowercase().as_str()),
        })
        .map(|p| p.name)
        .collect()
}

pub fn assert_game_closed(game_dir: &Path, exe: Option<&Path>) -> Result<()> {
    let running = running_in(game_dir, exe);
    if !running.is_empty() {
        return Err(fail("errGameRunning", running.join(", ")));
    }
    if let Some(e) = exe {
        if executable_locked(e) {
            return Err(fail("errGameRunning", e.display().to_string()));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Gpu {
    pub name: String,
    pub driver: String,
}

/// Queried through nvidia-smi; None without an NVIDIA driver.
pub fn gpu_info() -> Option<Vec<Gpu>> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("nvidia-smi.exe")
        .args(["--query-gpu=name,driver_version", "--format=csv,noheader"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let rows: Vec<Gpu> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut parts = l.split(',').map(|s| s.trim().to_string());
            Gpu { name: parts.next().unwrap_or_default(), driver: parts.next().unwrap_or_default() }
        })
        .collect();
    Some(rows)
}

/// Driver 616.64+ was measured upstream to fault inside NGX during neural
/// evaluates with the RenoDX DLSS 5 consumer (DLSS5-Feeder issue #54).
pub fn driver_neural_fault(rows: &[Gpu]) -> bool {
    rows.iter().any(|g| {
        let mut it = g.driver.split('.');
        let major: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minor: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        major * 100 + minor >= 61664
    })
}
