//! Headless commands, useful for scripting and diagnostics:
//!
//!   NeuralSwap scan    <game dir>
//!   NeuralSwap install <game dir> [--route native|renodx|feeder] [--exe <rel>] [--accept-anticheat]
//!   NeuralSwap restore <game dir>
//!   NeuralSwap library
//!   NeuralSwap payload
//!   NeuralSwap pack-release [out dir] [--notes "text"] [--no-payload]

use crate::core::apply::{self, InstallConfig, Route};
use crate::core::{history::HistoryStore, payload, scan, Event, Level};
use crate::state::{data_dir, Settings};
use std::path::PathBuf;

fn attach_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn print_event(e: Event) {
    let tag = match e.level {
        Level::Info => "  ",
        Level::Ok => "✔ ",
        Level::Warn => "! ",
        Level::Error => "✖ ",
    };
    println!("{tag}{}", e.text);
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// Returns Some(exit code) when the arguments named a CLI command.
pub fn run(args: &[String]) -> Option<i32> {
    let cmd = args.get(1)?.as_str();
    if !matches!(cmd, "scan" | "install" | "restore" | "library" | "payload" | "pack-release" | "import-payload" | "get-official" | "import-core" | "install-to" | "uninstall-now" | "--help" | "-h") {
        return None;
    }
    attach_console();
    println!();
    let settings = Settings::load();
    let code = match cmd {
        "scan" => {
            let Some(dir) = args.get(2).map(PathBuf::from) else { return Some(usage()) };
            let s = scan::scan_game(&dir);
            println!("{}", serde_json::to_string_pretty(&s).unwrap());
            0
        }
        "payload" => match payload::locate(settings.payload_dir.as_deref()) {
            Some(p) => {
                println!("{}", serde_json::to_string_pretty(&p).unwrap());
                0
            }
            None => {
                println!("No payload found. Tried: {:#?}", payload::candidates(settings.payload_dir.as_deref()));
                1
            }
        },
        "pack-release" => {
            let out = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("release"));
            let notes = flag(args, "--notes").unwrap_or_default();
            let publish = !args.iter().any(|a| a == "--no-payload");
            let p = payload::locate(settings.payload_dir.as_deref());
            if publish && p.is_none() {
                println!("No payload found.");
                return Some(1);
            }
            match crate::dist::pack_release(p.as_ref().map(|p| p.root.as_path()), publish, &out, &notes, &mut |l| println!("{l}")) {
                Ok(m) => {
                    println!("
Release v{} ready in {}", m.version, out.display());
                    println!("Create a GitHub release tagged v{} and attach the files in that folder (never the private folder).", m.version);
                    0
                }
                Err(e) => {
                    println!("FAILED: {e}");
                    2
                }
            }
        }
        "get-official" => {
            let Some(dest) = args.get(2).map(PathBuf::from) else { return Some(usage()) };
            let mut last = String::new();
            match crate::components::download_official(&dest, &mut |_, t| {
                if t != last {
                    if t.ends_with("ready") { println!("  {t}"); }
                    last = t;
                }
            }) {
                Ok(()) => { println!("Official components ready in {}", dest.display()); 0 }
                Err(e) => { println!("FAILED: {e}"); 2 }
            }
        }
        "import-core" => {
            let (Some(src), Some(dest)) = (args.get(2), args.get(3).map(PathBuf::from)) else { return Some(usage()) };
            let zip = if src.starts_with("http") {
                match crate::components::download_link(src, &mut |_, _| {}) { Ok(z) => z, Err(e) => { println!("FAILED: {e}"); return Some(2) } }
            } else { PathBuf::from(src) };
            match crate::components::import_core(&zip, &dest, args.iter().any(|a| a == "--allow-unknown"), flag(args, "--sha256").as_deref()) {
                Ok(found) => {
                    for f in found { println!("  {} {}", f.name, if f.known { "(known build)" } else { "(UNRECOGNISED build)" }); }
                    0
                }
                Err(e) => { println!("FAILED: {e}"); 2 }
            }
        }
        "install-to" => {
            let Some(dir) = args.get(2).map(PathBuf::from) else { return Some(usage()) };
            let dir = crate::setup::normalize_choice(&dir);
            match crate::setup::install(&dir, !args.iter().any(|a| a == "--no-desktop"), &mut |_, _| {}) {
                Ok(exe) => { println!("Installed {}", exe.display()); 0 }
                Err(e) => { println!("FAILED: {e}"); 2 }
            }
        }
        "uninstall-now" => match crate::setup::uninstall(args.iter().any(|a| a == "--remove-settings")) {
            Ok(()) => { println!("Uninstalled (files are removed once this process exits)"); 0 }
            Err(e) => { println!("FAILED: {e}"); 2 }
        },
        "import-payload" => {
            let Some(src) = args.get(2).map(PathBuf::from) else { return Some(usage()) };
            let dest = std::env::current_exe().unwrap().parent().unwrap().join("payload");
            match crate::ui::import_into(&src, &dest, &mut |_, _| {}) {
                Ok(()) => {
                    println!("Imported into {}", dest.display());
                    0
                }
                Err(e) => {
                    println!("FAILED: {e}");
                    2
                }
            }
        }
        "library" => {
            for g in crate::library::discover(&settings.manual_games, &settings.folders, &settings.excluded) {
                println!("{:<11} {:<8} {:<45} {}", g.launcher, g.id.clone().unwrap_or_default(), g.name, g.dir.display());
            }
            0
        }
        "install" => {
            let Some(dir) = args.get(2).map(PathBuf::from) else { return Some(usage()) };
            let Some(p) = payload::locate(settings.payload_dir.as_deref()) else {
                println!("No payload found.");
                return Some(1);
            };
            let s = scan::scan_game(&dir);
            let exe_rel = flag(args, "--exe");
            let Some(target) = s.candidate(exe_rel.as_deref()).cloned() else {
                println!("No game executable found in {}", dir.display());
                return Some(1);
            };
            let routes = apply::routes_for(target.bitness, target.api.as_deref(), target.api_label.as_deref());
            let route = flag(args, "--route")
                .and_then(|r| Route::from_key(&r))
                .or_else(|| apply::recommended_route(&routes, s.native_dlss_present(Some(&target)), s.install.as_ref().map(|i| i.route.as_str())));
            let Some(route) = route else {
                println!("No supported route for {} ({:?}, {}-bit)", target.rel, target.api_label, target.bitness);
                return Some(1);
            };
            println!("Game:   {}\nExe:    {}  ({}, {}-bit)\nRoute:  {}\n", dir.display(), target.rel, target.api_label.clone().unwrap_or_default(), target.bitness, route.label());
            let cfg = InstallConfig {
                game_dir: dir.clone(),
                exe_path: target.path.clone(),
                api: target.api.clone().unwrap_or_else(|| "dxgi".into()),
                api_label: target.api_label.clone(),
                bitness: target.bitness,
                route,
                anti_cheat_acknowledged: args.iter().any(|a| a == "--accept-anticheat"),
                payload: p,
                companions: settings.companions.clone(),
            };
            match apply::install(&cfg, &mut print_event) {
                Ok(m) => {
                    HistoryStore { file: data_dir().join("history.jsonl") }.record(&dir, Some(&m), "install", &s.game_name);
                    0
                }
                Err(e) => {
                    println!("\nFAILED [{}]: {e}", e.code);
                    2
                }
            }
        }
        "restore" => {
            let Some(dir) = args.get(2).map(PathBuf::from) else { return Some(usage()) };
            match apply::restore(&dir, &mut print_event) {
                Ok(m) => {
                    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    HistoryStore { file: data_dir().join("history.jsonl") }.record(&dir, m.as_ref(), if m.is_some() { "restore" } else { "recovery" }, &name);
                    0
                }
                Err(e) => {
                    println!("\nFAILED [{}]: {e}", e.code);
                    2
                }
            }
        }
        _ => usage(),
    };
    Some(code)
}

fn usage() -> i32 {
    println!(
        "NeuralSwap {}\n\n  scan    <game dir>\n  install <game dir> [--route native|renodx|feeder] [--exe <rel path>] [--accept-anticheat]\n  restore <game dir>\n  library\n  payload\n  import-payload <zip or folder>\n  get-official <payload dir>\n  import-core <zip or link> <payload dir> [--sha256 <hash>] [--allow-unknown]\n  install-to <folder> [--no-desktop]    (setup exe: silent install)
  uninstall-now [--remove-settings]
  pack-release [out dir] [--notes \"text\"] [--no-payload]\n\nWithout arguments the desktop app starts.",
        env!("CARGO_PKG_VERSION")
    );
    1
}
