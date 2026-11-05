// A release build is a GUI app on Windows: no console window beside it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    candlewick::run();
}
