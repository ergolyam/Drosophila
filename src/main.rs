#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod backend;
mod config;
mod discovery;
mod peer_probe;
mod privileged;
mod proxy;
mod tray;
#[cfg(windows)]
mod windows_console;

use tracing_subscriber::EnvFilter;

fn main() -> gtk::glib::ExitCode {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--peer-probe")) && args.next().is_none()
    {
        return if peer_probe::run() {
            gtk::glib::ExitCode::SUCCESS
        } else {
            gtk::glib::ExitCode::FAILURE
        };
    }

    let mut arguments = Vec::new();
    let mut debug = false;
    for argument in std::env::args() {
        if argument == "--debug" {
            debug = true;
        } else {
            arguments.push(argument);
        }
    }

    #[cfg(windows)]
    let has_console = windows_console::initialize(debug);

    let default_filter = if debug { "debug" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)),
        )
        .with_ansi(!cfg!(windows))
        .with_target(false)
        .try_init()
        .ok();

    #[cfg(target_os = "linux")]
    if debug {
        gtk::glib::log_set_writer_func(gtk::glib::log_writer_standard_streams);
    }

    #[cfg(windows)]
    if has_console {
        windows_console::redirect_glib_logs();
    }

    if let Some(worker) = privileged::WorkerArguments::parse(&arguments) {
        return match worker.and_then(privileged::run_worker) {
            Ok(()) => gtk::glib::ExitCode::SUCCESS,
            Err(error) => {
                tracing::error!(%error, "privileged TUN worker failed");
                gtk::glib::ExitCode::FAILURE
            }
        };
    }

    #[cfg(target_os = "linux")]
    if std::env::var_os("PKEXEC_UID").is_some() {
        tracing::error!("refusing to start the graphical application through pkexec");
        return gtk::glib::ExitCode::FAILURE;
    }

    let mut start_in_tray = false;
    let mut filtered_arguments = Vec::with_capacity(arguments.len());
    for (index, argument) in arguments.into_iter().enumerate() {
        if index != 0 && argument == "--start-in-tray" {
            start_in_tray = true;
        } else {
            filtered_arguments.push(argument);
        }
    }

    app::run(&filtered_arguments, start_in_tray)
}
