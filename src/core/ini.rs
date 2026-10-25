//! Line-preserving INI edits for ReShade.ini, presets and dlss5-feed.cfg.
//! Unknown keys, comments and ordering are always kept.

use regex::Regex;
use std::path::{Path, PathBuf};

fn is_header(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('[') && t.ends_with(']') && t.len() > 2
}

/// (start, end): start is the header line (or None for the root area before
/// any section), end is one past the last line of the section.
fn section_bounds(lines: &[String], section: &str) -> Option<(Option<usize>, usize)> {
    if section.is_empty() {
        let end = lines.iter().position(|l| is_header(l)).unwrap_or(lines.len());
        return Some((None, end));
    }
    let header = format!("[{section}]").to_lowercase();
    let start = lines.iter().position(|l| l.trim().to_lowercase() == header)?;
    let end = lines[start + 1..].iter().position(|l| is_header(l)).map(|i| start + 1 + i).unwrap_or(lines.len());
    Some((Some(start), end))
}

fn split_lines(text: &str) -> Vec<String> {
    text.trim_start_matches('\u{feff}').split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect()
}

fn key_of(line: &str) -> Option<String> {
    let t = line.trim_start();
    if t.starts_with(';') || t.starts_with('#') {
        return None;
    }
    let (k, _) = t.split_once('=')?;
    Some(k.trim().to_lowercase())
}

pub fn get(text: &str, section: &str, key: &str) -> Option<String> {
    let lines = split_lines(text);
    let (start, end) = section_bounds(&lines, section)?;
    let wanted = key.to_lowercase();
    let from = start.map(|s| s + 1).unwrap_or(0);
    for line in &lines[from..end] {
        if key_of(line).as_deref() == Some(wanted.as_str()) {
            return Some(line.split_once('=').map(|(_, v)| v.trim().to_string()).unwrap_or_default());
        }
    }
    None
}

pub fn set(text: &str, section: &str, key: &str, value: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines = split_lines(text);
    if lines.len() == 1 && lines[0].is_empty() {
        lines.clear();
    }
    match section_bounds(&lines, section) {
        None => {
            if lines.last().map(|l| !l.is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
            lines.push(format!("[{section}]"));
            lines.push(format!("{key}={value}"));
        }
        Some((start, end)) => {
            let wanted = key.to_lowercase();
            let from = start.map(|s| s + 1).unwrap_or(0);
            let mut changed = false;
            for i in from..end {
                if key_of(&lines[i]).as_deref() == Some(wanted.as_str()) {
                    let (lhs, _) = lines[i].split_once('=').unwrap();
                    lines[i] = format!("{lhs}={value}");
                    changed = true;
                    break;
                }
            }
            if !changed {
                // Insert before trailing blank lines of the section.
                let mut at = end;
                while at > from && lines[at - 1].trim().is_empty() && start.is_some() {
                    at -= 1;
                }
                lines.insert(at, format!("{key}={value}"));
            }
        }
    }
    let mut out = lines.join(newline);
    while out.ends_with(newline) {
        out.truncate(out.len() - newline.len());
    }
    out.push_str(newline);
    out
}

fn merge_named_list(value: Option<&str>, required: &[String]) -> String {
    let name = |s: &str| s.split('=').next().unwrap_or("").trim().to_lowercase();
    let req: Vec<String> = required.iter().map(|r| name(r)).collect();
    let mut out: Vec<String> = required.to_vec();
    for item in value.unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !req.contains(&name(item)) {
            out.push(item.to_string());
        }
    }
    out.join(",")
}

/// ReShade Setup may seed `Shaders\**\**`, which Windows rejects, so the
/// canonical recursive path goes first and malformed copies are collapsed.
fn configure_search_path(value: Option<&str>, required: &str) -> String {
    let wild = Regex::new(r"(?:\\\*\*){2,}$").unwrap();
    let canonical = |s: &str| wild.replace(&s.trim().replace('/', "\\"), "\\**").to_string();
    let base = |s: &str| canonical(s).trim_end_matches("\\**").trim_end_matches('\\').to_lowercase();
    let req = base(required);
    let mut out = vec![required.to_string()];
    for item in value.unwrap_or("").split(',').map(canonical).filter(|s| !s.is_empty()) {
        if base(&item) != req {
            out.push(item);
        }
    }
    out.join(",")
}

pub fn configure_game_reshade(text: &str, provider: u32) -> String {
    let mut out = text.to_string();
    let p = configure_search_path(get(&out, "GENERAL", "EffectSearchPaths").as_deref(), ".\\reshade-shaders\\Shaders\\**");
    out = set(&out, "GENERAL", "EffectSearchPaths", &p);
    let p = configure_search_path(get(&out, "GENERAL", "TextureSearchPaths").as_deref(), ".\\reshade-shaders\\Textures\\**");
    out = set(&out, "GENERAL", "TextureSearchPaths", &p);
    let preset = get(&out, "GENERAL", "PresetPath").filter(|s| !s.is_empty()).unwrap_or_else(|| ".\\ReShadePreset.ini".into());
    out = set(&out, "GENERAL", "PresetPath", &preset);
    out = set(&out, "GENERAL", "StartupPresetPath", "");
    out = set(&out, "GENERAL", "NoReloadOnInit", "0");
    let defs = merge_named_list(get(&out, "GENERAL", "PreprocessorDefinitions").as_deref(), &[format!("DLSS5_MV_PROVIDER={provider}")]);
    out = set(&out, "GENERAL", "PreprocessorDefinitions", &defs);
    set(&out, "ADDON", "AddonPath", ".\\")
}

pub fn configure_consumer(text: &str) -> String {
    let mut out = text.to_string();
    for (k, v) in [("EnableHooks", "2"), ("NeuralUplift", "1"), ("NREnableUpscaling", "0")] {
        out = set(&out, "RenoDX.DLSS5", k, v);
    }
    if get(&out, "RenoDX.DLSS5", "NRStyle").as_deref() == Some("2") {
        out = set(&out, "RenoDX.DLSS5", "NRStyle", "0");
    }
    out
}

pub fn configure_preset(text: &str, provider: u32) -> String {
    let mut out = text.to_string();
    let provider_technique = if provider == 3 { "Lumenite_Kernel@lumenite_Kernel.fx" } else { "vort_MotionEffects@vort_Motion.fx" };
    let ours = ["lumenite_kernel", "lumenite_quantmotion", "vort_motioneffects", "dlss5_feed", "launchpad"];
    for key in ["Techniques", "TechniqueSorting"] {
        let mut list = vec![provider_technique.to_string(), "DLSS5_Feed@DLSS5_Feed.fx".to_string()];
        for item in get(&out, "", key).unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let name = item.split('@').next().unwrap_or("").trim().to_lowercase();
            if !ours.contains(&name.as_str()) {
                list.push(item.to_string());
            }
        }
        out = set(&out, "", key, &list.join(","));
    }
    let req = [format!("DLSS5_MV_PROVIDER={provider}")];
    let defs = merge_named_list(get(&out, "", "PreprocessorDefinitions").as_deref(), &req);
    out = set(&out, "", "PreprocessorDefinitions", &defs);
    let defs = merge_named_list(get(&out, "DLSS5_Feed.fx", "PreprocessorDefinitions").as_deref(), &req);
    out = set(&out, "DLSS5_Feed.fx", "PreprocessorDefinitions", &defs);
    let mut crlf = out.replace("\r\n", "\n").replace('\n', "\r\n");
    while crlf.ends_with("\r\n\r\n") {
        crlf.truncate(crlf.len() - 2);
    }
    if !crlf.ends_with("\r\n") {
        crlf.push_str("\r\n");
    }
    crlf
}

const FEED_DEFAULTS: &[(&str, &str)] = &[
    ("enabled", "1"), ("mode", "2"), ("hdr", "-1"), ("depth_inverted", "-1"), ("flags", "-1"),
    ("reset_every", "0"), ("warmup_rebuild", "180"), ("rebuild", "0"), ("log_frames", "3"),
    ("create_delay", "60"), ("preset", "0"), ("work_resolution", "100"),
    ("mv_scale_x", "1.000"), ("mv_scale_y", "1.000"), ("host_window", "0"), ("async_home", "1"),
];

pub fn configure_feed(text: &str) -> String {
    let mut seen = Vec::new();
    let mut out: Vec<String> = split_lines(text)
        .into_iter()
        .map(|line| {
            let t = line.trim_start();
            if t.starts_with('#') || t.starts_with(';') {
                return line;
            }
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim().to_lowercase();
                if FEED_DEFAULTS.iter().any(|(d, _)| *d == key) {
                    seen.push(key.clone());
                    return format!("{key}={}", v.trim());
                }
            }
            line
        })
        .collect();
    while out.last().map(|l| l.is_empty()).unwrap_or(false) {
        out.pop();
    }
    for (k, v) in FEED_DEFAULTS {
        if !seen.iter().any(|s| s == k) {
            out.push(format!("{k}={v}"));
        }
    }
    out.join("\r\n") + "\r\n"
}

pub fn preset_path(exe_dir: &Path, reshade_ini: &str) -> PathBuf {
    let value = get(reshade_ini, "GENERAL", "PresetPath").filter(|s| !s.is_empty()).unwrap_or_else(|| ".\\ReShadePreset.ini".into());
    let v = value.trim_start_matches(".\\").trim_start_matches("./");
    let p = Path::new(v);
    if p.is_absolute() { p.to_path_buf() } else { exe_dir.join(p) }
}

/// UTF-8 (with or without BOM) or UTF-16LE with BOM; empty when unreadable.
pub fn read_text(file: &Path) -> String {
    let Ok(bytes) = std::fs::read(file) else { return String::new() };
    if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] == 0xfe {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get_round_trip() {
        let t = set("", "GENERAL", "A", "1");
        assert_eq!(get(&t, "GENERAL", "A").as_deref(), Some("1"));
        let t = set(&t, "GENERAL", "A", "2");
        assert_eq!(get(&t, "general", "a").as_deref(), Some("2"));
        let t = set(&t, "", "Techniques", "x");
        assert_eq!(get(&t, "", "Techniques").as_deref(), Some("x"));
    }

    #[test]
    fn search_path_collapses_wildcards() {
        let p = configure_search_path(Some(".\\reshade-shaders\\Shaders\\**\\**,C:\\Mine"), ".\\reshade-shaders\\Shaders\\**");
        assert_eq!(p, ".\\reshade-shaders\\Shaders\\**,C:\\Mine");
    }
}
