// Suppresses the console window Windows otherwise attaches to a "console
// subsystem" binary -- without this, a released .exe pops up a terminal
// alongside the app window, and closing that terminal kills the app with
// it (the console is the process's controlling window in that subsystem).
// Left enabled in debug builds so `cargo run`/`tauri dev` still show
// println!/panic output in a console during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    open_mind_ai_lib::run();
}
