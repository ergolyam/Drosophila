#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod backend;
mod config;
mod discovery;
#[cfg(feature = "tun")]
mod privileged;
#[cfg(not(feature = "tun"))]
#[path = "privileged_disabled.rs"]
mod privileged;
mod proxy;
mod system_proxy;
#[cfg(windows)]
mod windows_console;

use tracing_subscriber::EnvFilter;

fn main() -> gtk::glib::ExitCode {
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

    #[cfg(windows)]
    if has_console {
        windows_console::redirect_glib_logs();
    }

    #[cfg(feature = "tun")]
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

    app::run(&arguments)
}
