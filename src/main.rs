#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod assets;
mod audio;
mod config;
mod diagnostics;
mod engine;
mod hotkey;
mod input;
mod ui;
mod worker;
fn main() {
    let home_for_panic = config::home();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::fs::create_dir_all(home_for_panic.join("reports"));
        let _ = std::fs::write(home_for_panic.join("reports/crash.txt"), format!("{info}"));
    }));
    let home = config::home();
    let args: Vec<String> = std::env::args().collect();
    let outcome = if args.iter().any(|a| a == "--self-test") {
        diagnostics::run(&home, &args).map(|_| ())
    } else if args.iter().any(|a| a == "--pipeline-probe") {
        diagnostics::pipeline_probe(&home).map(|_| ())
    } else if args.iter().any(|a| a == "--hook-lifecycle-probe") {
        diagnostics::hook_lifecycle_probe(&home).map(|_| ())
    } else if args.iter().any(|a| a == "--hotkey-probe") {
        diagnostics::hotkey_probe(&home).map(|_| ())
    } else if args.iter().any(|a| a == "--input-probe") {
        diagnostics::input_probe(&home).map(|_| ())
    } else {
        use std::hash::{Hash, Hasher};
        use windows_sys::Win32::{Foundation::*, System::Threading::*, UI::WindowsAndMessaging::*};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        home.to_string_lossy().to_lowercase().hash(&mut hasher);
        let name = input::wide(&format!("Local\\FastASR-{:x}", hasher.finish()));
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if !mutex.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
                let hwnd = FindWindowW(input::wide("FastAsrOverlay").as_ptr(), std::ptr::null());
                if !hwnd.is_null() {
                    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
                CloseHandle(mutex);
                return;
            }
            let result = ui::run(home.clone(), args.iter().any(|a| a == "--ui-probe"));
            if !mutex.is_null() {
                CloseHandle(mutex);
            }
            result
        }
    };
    if let Err(error) = outcome {
        if args.len() > 1 {
            let _ = std::fs::create_dir_all(home.join("reports"));
            let _ = std::fs::write(home.join("reports/diagnostic-error.txt"), &error);
            std::process::exit(1);
        }
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                input::wide(&error).as_ptr(),
                input::wide("Fast ASR").as_ptr(),
                0x10,
            );
        }
    }
}
