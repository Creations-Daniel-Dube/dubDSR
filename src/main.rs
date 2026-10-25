#![cfg_attr(windows, windows_subsystem = "windows")]

mod error;
mod gpu;
mod icon;
mod i18n;
mod pipeline;
mod settings;
mod ui;

fn main() -> eframe::Result {
    ui::run()
}
