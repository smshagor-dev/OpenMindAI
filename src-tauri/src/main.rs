// Suppresses the console window Windows otherwise attaches to a "console
// subsystem" binary -- without this, a released .exe pops up a terminal
// alongside the app window, and closing that terminal kills the app with
// it (the console is the process's controlling window in that subsystem).
// Left enabled in debug builds so `cargo run`/`tauri dev` still show
// println!/panic output in a console during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(target_os = "windows", not(debug_assertions)))]
use std::{thread, time::Duration};

#[cfg(all(target_os = "windows", not(debug_assertions)))]
use windows::{
    core::w,
    Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE},
    },
};

const BACKGROUND_ARG: &str = "--background";
const BACKGROUND_ENV: &str = "OPENMINDAI_BACKGROUND_BOOT";

#[cfg(all(target_os = "windows", not(debug_assertions)))]
struct SingleInstanceGuard(HANDLE);

#[cfg(all(target_os = "windows", not(debug_assertions)))]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn acquire_single_instance(background_launch: bool) -> Option<SingleInstanceGuard> {
    let mutex = unsafe {
        CreateMutexW(
            None,
            false,
            w!("Local\\OpenMindAI.Desktop.SingleInstance.v3"),
        )
    }
    .ok()?;
    let already_running = unsafe { GetLastError() == ERROR_ALREADY_EXISTS };

    if !already_running {
        return Some(SingleInstanceGuard(mutex));
    }

    // A Windows-login background launch must stay silent if another instance
    // is already alive. A normal user launch instead hands off to the existing
    // hidden/prepared process so its loaded model and runtime are reused.
    if !background_launch {
        for _ in 0..20 {
            if let Ok(window) = unsafe { FindWindowW(None, w!("OpenMindAI")) } {
                if !window.is_invalid() {
                    unsafe {
                        let _ = ShowWindow(window, SW_RESTORE);
                        let _ = SetForegroundWindow(window);
                    }
                    break;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    unsafe {
        let _ = CloseHandle(mutex);
    }
    None
}

fn main() {
    let background_launch = std::env::args().any(|arg| arg == BACKGROUND_ARG);
    if background_launch {
        std::env::set_var(BACKGROUND_ENV, "1");
    }

    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    let _single_instance = match acquire_single_instance(background_launch) {
        Some(guard) => guard,
        None => return,
    };

    open_mind_ai_lib::run();
}
