mod app;
mod backend;
mod config;
mod discovery;
mod proxy;

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

    let default_filter = if debug { "debug" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)),
        )
        .with_target(false)
        .try_init()
        .ok();

    app::run(&arguments)
}
