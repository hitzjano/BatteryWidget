//! Minimal file logger (`app.log` next to the config). No external logging framework:
//! the application writes a handful of lines per session.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{LevelFilter, Log, Metadata, Record};

/// The log starts over once it has grown beyond this size.
const MAX_SIZE: u64 = 512 * 1024;

struct FileLogger(Mutex<File>);

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let thread = std::thread::current();
        let line = format!(
            "{seconds} {:5} [{}] {}: {}\n",
            record.level(),
            thread.name().unwrap_or("?"),
            record.target(),
            record.args()
        );
        if let Ok(mut file) = self.0.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {}
}

/// Start logging to `path`. Logging is best effort: if the file cannot be opened the
/// application simply runs without a log.
pub fn init(path: &Path, verbose: bool) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let too_big = std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_SIZE);
    let file = OpenOptions::new()
        .create(true)
        .append(!too_big)
        .write(true)
        .truncate(too_big)
        .open(path);
    let Ok(file) = file else { return };
    if log::set_boxed_logger(Box::new(FileLogger(Mutex::new(file)))).is_ok() {
        log::set_max_level(if verbose {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        });
    }
    // Panics in worker threads would otherwise vanish: a GUI program has no console.
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
}
