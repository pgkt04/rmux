// Ported from tmux log.c @ 8f25579c
//! Debug log file, `log_debug`, `fatal` and `fatalx`.

use std::backtrace::Backtrace;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::vis::{VisFlags, strvis};

/// Log verbosity; 0 means off.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct LogLevel(pub u32);

static LEVEL: AtomicU32 = AtomicU32::new(0);
static FILE: Mutex<Option<File>> = Mutex::new(None);

struct CrashContext {
    directory: PathBuf,
    role: &'static str,
}

static CRASH: OnceLock<CrashContext> = OnceLock::new();

pub fn init_crash_reporting(role: &'static str) {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".local/state"))
        });
    let Some(state) = state else {
        return;
    };
    if CRASH
        .set(CrashContext {
            directory: state.join("rmux"),
            role,
        })
        .is_err()
    {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crash_report(format_args!("{info}"));
        previous(info);
    }));
}

fn crash_report(reason: fmt::Arguments<'_>) {
    let Some(context) = CRASH.get() else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let path = context.directory.join(format!(
        "rmux-crash-{}-{}-{}.log",
        context.role,
        std::process::id(),
        now.as_nanos()
    ));
    let result = (|| -> std::io::Result<()> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&context.directory)?;
        let metadata = std::fs::symlink_metadata(&context.directory)?;
        if !metadata.is_dir() || metadata.uid() != rmux_sys::proc::getuid().0 {
            return Err(std::io::Error::other(
                "crash directory is not an owned directory",
            ));
        }
        let mut report = File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        writeln!(
            report,
            "rmux {} {} pid {} at {}.{:06}",
            env!("CARGO_PKG_VERSION"),
            context.role,
            std::process::id(),
            now.as_secs(),
            now.subsec_micros()
        )?;
        writeln!(report, "executable: {:?}", std::env::current_exe())?;
        writeln!(report, "working directory: {:?}", std::env::current_dir())?;
        writeln!(
            report,
            "{reason}\nbacktrace:\n{}",
            Backtrace::force_capture()
        )?;
        report.sync_all()
    })();
    match result {
        Ok(()) => {
            let _ = writeln!(
                std::io::stderr().lock(),
                "rmux: crash report saved to {}",
                path.display()
            );
        }
        Err(error) => {
            let _ = writeln!(
                std::io::stderr().lock(),
                "rmux: could not save crash report to {}: {error}\n{reason}\n{}",
                path.display(),
                Backtrace::force_capture()
            );
        }
    }
}

fn file() -> std::sync::MutexGuard<'static, Option<File>> {
    FILE.lock().unwrap_or_else(|e| e.into_inner())
}

/// `log_add_level`
pub fn add_level() {
    LEVEL.fetch_add(1, Ordering::Relaxed);
}

/// `log_get_level`
pub fn level() -> LogLevel {
    LogLevel(LEVEL.load(Ordering::Relaxed))
}

/// Log file name for `name`: `rmux-<name>-<pid>.log` (Main policy, section 2.7).
pub fn file_name(name: &str) -> String {
    format!("rmux-{}-{}.log", name, std::process::id())
}

/// `log_open`: no file at level 0; a failed open keeps the level and leaves no file.
pub fn open(name: &str) {
    if LEVEL.load(Ordering::Relaxed) == 0 {
        return;
    }
    close();
    let opened = File::options()
        .append(true)
        .create(true)
        .open(file_name(name))
        .ok();
    *file() = opened;
}

/// `log_toggle`
pub fn toggle(name: &str) {
    if LEVEL.load(Ordering::Relaxed) == 0 {
        LEVEL.store(1, Ordering::Relaxed);
        open(name);
        write("", format_args!("log opened"));
    } else {
        write("", format_args!("log closed"));
        LEVEL.store(0, Ordering::Relaxed);
        close();
    }
}

/// `log_close`
pub fn close() {
    *file() = None;
}

/// `log_file != NULL`
pub fn enabled() -> bool {
    file().is_some()
}

/// `log_vwrite`: `<sec>.<usec> <prefix><message>\n`, message through raw
/// `stravis(VIS_OCTAL|VIS_CSTYLE|VIS_TAB|VIS_NL)`.
pub fn write(prefix: &str, args: fmt::Arguments<'_>) {
    if !enabled() {
        return;
    }
    let message = fmt::format(args);
    write_bytes(prefix, message.as_bytes());
}

pub fn write_bytes(prefix: &str, message: &[u8]) {
    if !enabled() {
        return;
    }
    let mut line = Vec::with_capacity(message.len() * 2 + prefix.len() + 32);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let _ = std::io::Write::write_fmt(
        &mut line,
        format_args!("{}.{:06} {}", now.as_secs(), now.subsec_micros(), prefix),
    );
    strvis(
        &mut line,
        message,
        VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB | VisFlags::NL,
    );
    line.push(b'\n');
    if let Some(f) = file().as_mut() {
        if f.write_all(&line).is_ok() {
            let _ = f.flush();
        }
    }
}

#[doc(hidden)]
pub fn fatal_errno() -> i32 {
    rmux_sys::errno()
}

/// `fatal`: the macro captures errno before evaluating its arguments.
pub fn fatal_with(errno: i32, args: fmt::Arguments<'_>) -> ! {
    let mut prefix = b"fatal: ".to_vec();
    prefix.extend_from_slice(&rmux_sys::strerror(errno));
    prefix.extend_from_slice(b": ");
    prefix.truncate(255);
    crash_report(format_args!("{}{args}", String::from_utf8_lossy(&prefix)));
    write(&String::from_utf8_lossy(&prefix), args);
    std::process::exit(1)
}

/// `fatalx`
pub fn fatalx_with(args: fmt::Arguments<'_>) -> ! {
    crash_report(format_args!("fatal: {args}"));
    write("fatal: ", args);
    std::process::exit(1)
}

/// `log_debug(fmt, ...)`. Arguments are evaluated like C call arguments;
/// guard costly ones with `log::level() != LogLevel(0)` at the call site.
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::log::write("", format_args!($($arg)*))
    };
}

/// `fatal(fmt, ...)`: `fatal: <strerror(errno)>: <message>`, exit 1.
#[macro_export]
macro_rules! fatal {
    ($($arg:tt)*) => {{
        let errno = $crate::log::fatal_errno();
        $crate::log::fatal_with(errno, format_args!($($arg)*))
    }};
}

/// `fatalx(fmt, ...)`: `fatal: <message>`, exit 1.
#[macro_export]
macro_rules! fatalx {
    ($($arg:tt)*) => {
        $crate::log::fatalx_with(format_args!($($arg)*))
    };
}
