#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod hook;
#[cfg(any(windows, test))]
mod icon;
#[cfg(any(windows, test))]
mod model;
#[cfg(any(windows, test))]
mod protocol;
#[cfg(any(windows, test))]
mod style;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn main() {
    if let Err(error) = windows::run() {
        windows::show_error(&error.to_string());
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Floating Tray runs on Windows 11 x64.");
}
