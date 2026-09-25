mod app;
mod config;
mod geometry;
mod ipc;
mod note;
mod outputs;
mod store;
mod theme;
mod widgets;
mod xdg;

use std::path::PathBuf;

use clap::Parser;
use flexi_logger::{LogSpecBuilder, Logger};

#[derive(Parser, Debug)]
#[command(
    version = concat!(env!("CARGO_PKG_VERSION")),
    about = env!("CARGO_PKG_DESCRIPTION")
)]
struct Args {
    /// Path to the configuration file.
    #[arg(short, long, value_parser = clap::value_parser!(PathBuf))]
    config_path: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Send a message to a running layernotes instance
    Msg {
        #[command(subcommand)]
        command: ipc::IpcCommand,
    },
}

fn main() -> iced::Result {
    let args = Args::parse();

    if let Some(Command::Msg { command }) = &args.command {
        if let Err(e) = ipc::run_client(command) {
            eprintln!("Error: {e:#}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    let _logger = Logger::with(
        LogSpecBuilder::new()
            .default(log::LevelFilter::Info)
            .build(),
    )
    .start();

    let (config, config_path) = config::get_config(args.config_path).unwrap_or_else(|e| {
        eprintln!("Failed to read config: {e:#}");
        std::process::exit(1);
    });

    let layer = config.layer;

    iced::application(
        move || app::App::new(config, config_path),
        app::App::update,
        app::App::view,
    )
    .layer_shell(outputs::Outputs::settings(layer, None))
    .subscription(app::App::subscription)
    .theme(app::App::theme)
    .scale_factor(app::App::scale_factor)
    .run()
}
