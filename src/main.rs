#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(all(windows, target_arch = "x86_64"))]
fn main() {
    if let Err(error) = quickcap::windows::run() {
        if std::env::args_os().len() == 1 {
            quickcap::windows::show_error(&format!("{error:#}"));
        } else {
            eprintln!("{error:#}");
        }
        std::process::exit(1);
    }
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn main() {
    eprintln!("quickcap only runs on Windows x64. Core tests can run on this host.");
    std::process::exit(1);
}
