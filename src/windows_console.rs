use std::io::{self, Write};

use gtk::glib::{self, LogLevel, LogWriterOutput};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole, GetConsoleCP,
};

pub(crate) fn initialize(debug: bool) -> bool {
    // A GUI-subsystem executable starts without a console. If its parent owns
    // one (for example, PowerShell or cmd.exe), reuse it for log output. When a
    // debug launch has no parent console, create one so a shortcut with
    // `--debug` still displays logs.
    #[allow(unsafe_code)]
    unsafe {
        // SAFETY: ATTACH_PARENT_PROCESS is a documented sentinel value and the
        // calls do not dereference pointers. A failed attachment requires no
        // cleanup, and AllocConsole is only attempted for a debug launch.
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 && debug {
            AllocConsole();
        }

        GetConsoleCP() != 0
    }
}

pub(crate) fn redirect_glib_logs() {
    glib::log_set_writer_func(|level, fields| {
        if glib_level_enabled(level) {
            let message = glib::log_writer_format_fields(level, fields, false);
            write_console(&message);
        }

        LogWriterOutput::Handled
    });

    glib::set_print_handler(|message| {
        if tracing::enabled!(target: "glib", tracing::Level::INFO) {
            write_console(message);
        }
    });
    glib::set_printerr_handler(|message| {
        if tracing::enabled!(target: "glib", tracing::Level::WARN) {
            write_console(message);
        }
    });
}

fn glib_level_enabled(level: LogLevel) -> bool {
    match level {
        LogLevel::Error | LogLevel::Critical => {
            tracing::enabled!(target: "glib", tracing::Level::ERROR)
        }
        LogLevel::Warning => tracing::enabled!(target: "glib", tracing::Level::WARN),
        LogLevel::Message | LogLevel::Info => {
            tracing::enabled!(target: "glib", tracing::Level::INFO)
        }
        LogLevel::Debug => tracing::enabled!(target: "glib", tracing::Level::DEBUG),
    }
}

fn write_console(message: &str) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "{}", message.trim_end());
}
