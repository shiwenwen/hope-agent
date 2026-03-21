// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Maximum consecutive crash restarts before giving up
const MAX_CRASH_RESTARTS: u32 = 3;

fn main() {
    let mut crash_count: u32 = 0;

    loop {
        let result = std::panic::catch_unwind(|| {
            app_lib::run();
        });

        match result {
            Ok(_) => {
                // Normal exit (user closed window / quit), don't restart
                break;
            }
            Err(panic_info) => {
                crash_count += 1;
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic".to_string()
                };
                eprintln!(
                    "[OpenComputer] Crash detected ({}/{}): {}",
                    crash_count, MAX_CRASH_RESTARTS, msg
                );

                if crash_count >= MAX_CRASH_RESTARTS {
                    eprintln!(
                        "[OpenComputer] Max crash restarts reached ({}), exiting.",
                        MAX_CRASH_RESTARTS
                    );
                    std::process::exit(1);
                }

                // Brief delay before restart to avoid tight crash loops
                std::thread::sleep(std::time::Duration::from_secs(1));
                eprintln!("[OpenComputer] Restarting...");
            }
        }
    }
}
