#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod art;
mod cli;
mod components;
mod core;
mod dist;
#[macro_use]
mod i18n;
mod i18n_table;
mod library;
mod magpie;
mod setup;
mod state;
mod ui;
mod update;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    if args.iter().any(|a| a == "--uninstall") {
        return ui::installer::run(ui::installer::Mode::Uninstall);
    }
    if setup::should_show_installer() {
        return ui::installer::run(ui::installer::Mode::Install);
    }
    setup::cleanup_old();
    setup::refresh_registration();
    ui::run()
}
