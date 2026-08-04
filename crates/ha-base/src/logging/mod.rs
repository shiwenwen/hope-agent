mod app_logger;
mod config;
mod db;
mod file_ops;
mod file_writer;
mod types;

pub use app_logger::AppLogger;
pub use config::{db_path, load_log_config, save_log_config};
pub use db::LogDB;
pub use file_ops::*;
pub use types::*;

// ── Global Logging Macros ────────────────────────────────────────
//
// Use these macros instead of `log::info!` / `log::warn!` etc. so that
// messages are written to both the SQLite database AND the log file via
// `AppLogger`.  The `log` crate only prints to the console (stderr).
//
// Usage:
//   app_info!("category", "source", "message {} {}", arg1, arg2);
//   app_warn!("category", "source", "something went wrong: {}", err);
//   app_error!("category", "source", "fatal: {}", err);
//   app_debug!("category", "source", "verbose detail: {}", val);

#[macro_export]
macro_rules! app_info {
    ($cat:expr, $src:expr, $($arg:tt)+) => {
        if let Some(logger) = $crate::get_logger() {
            logger.log("info", $cat, $src, &format!($($arg)+), None, None, None);
        }
    };
}

#[macro_export]
macro_rules! app_warn {
    ($cat:expr, $src:expr, $($arg:tt)+) => {
        if let Some(logger) = $crate::get_logger() {
            logger.log("warn", $cat, $src, &format!($($arg)+), None, None, None);
        }
    };
}

#[macro_export]
macro_rules! app_error {
    ($cat:expr, $src:expr, $($arg:tt)+) => {
        if let Some(logger) = $crate::get_logger() {
            logger.log("error", $cat, $src, &format!($($arg)+), None, None, None);
        }
    };
}

#[macro_export]
macro_rules! app_debug {
    ($cat:expr, $src:expr, $($arg:tt)+) => {
        if let Some(logger) = $crate::get_logger() {
            logger.log("debug", $cat, $src, &format!($($arg)+), None, None, None);
        }
    };
}
