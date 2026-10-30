#![cfg_attr(windows, windows_subsystem = "windows")]

mod error;
mod gpu;
mod icon;
mod i18n;
mod pipeline;
mod settings;
mod ui;

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("probe") {
        let (text, code) = gpu::probe_report();
        write_console(&text);
        std::process::exit(code);
    }
    ui::run()
}

fn write_console(text: &str) {
    use std::io::Write;
    let mut stdout = std::io::stdout();
    if writeln!(stdout, "{text}").is_ok() && stdout.flush().is_ok() {
        return;
    }
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn AttachConsole(pid: u32) -> i32;
        }
        unsafe {
            AttachConsole(u32::MAX);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
            let _ = writeln!(file, "{text}");
            let _ = file.flush();
        }
    }
}
