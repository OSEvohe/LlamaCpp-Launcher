use std::sync::{Arc, RwLock};

use clap::Parser;
use llama_launcher::server;
use llama_launcher::server::SharedState;
use llama_launcher::service::LlamaLauncherService;
use llama_launcher::service_install::{install_service, install_task, uninstall_service, uninstall_task};
use tokio::net::TcpListener;

#[derive(Debug, Parser)]
#[command(
    about = "LLama Launcher - API server and web dashboard for llama.cpp",
    long_about = None
)]
struct Cli {
    #[arg(
        long,
        help = "API server bind address (default: from settings or 127.0.0.1)"
    )]
    api_host: Option<String>,
    #[arg(
        long,
        help = "API server bind port (default: from settings or 7890)"
    )]
    api_port: Option<i64>,
    #[arg(long, help = "Install a logon scheduled task")]
    install_task: bool,
    #[arg(long, help = "Uninstall the logon scheduled task")]
    uninstall_task: bool,
    #[arg(long, help = "Overwrite existing scheduled task when installing")]
    force: bool,
    #[arg(long, help = "Install as a native Windows service")]
    install_service: bool,
    #[arg(long, help = "Uninstall the native Windows service")]
    uninstall_service: bool,
}

fn resolve_api_settings(cli_host: Option<&str>, cli_port: Option<i64>) -> (String, i64) {
    let service = LlamaLauncherService::new(None);
    let settings = service.load_global();

    let host = cli_host
        .filter(|h| !h.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            if settings.api_host.trim().is_empty() {
                None
            } else {
                Some(settings.api_host)
            }
        })
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let mut port = cli_port.unwrap_or(settings.api_port);
    if !(0..=65535).contains(&port) {
        port = 0;
    }
    if port <= 0 && cli_port != Some(0) {
        port = 7890;
    }

    (host, port)
}

fn handle_install_task(force: bool) {
    match install_task(force) {
        Ok(()) => println!("Scheduled task 'LLama Launcher' installed"),
        Err(err) => {
            eprintln!("Error: {}", err);
            std::process::exit(1);
        }
    }
}

fn handle_uninstall_task() {
    match uninstall_task() {
        Ok(()) => println!("Scheduled task 'LLama Launcher' uninstalled"),
        Err(err) => {
            eprintln!("Error: {}", err);
            std::process::exit(1);
        }
    }
}

fn handle_install_service() {
    match install_service() {
        Ok(()) => println!("Windows service 'LlamaLauncher' installed"),
        Err(err) => {
            eprintln!("Error: {}", err);
            std::process::exit(1);
        }
    }
}

fn handle_uninstall_service() {
    match uninstall_service() {
        Ok(()) => println!("Windows service 'LlamaLauncher' uninstalled"),
        Err(err) => {
            eprintln!("Error: {}", err);
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Cli::parse();

    let actions = [
        args.install_task,
        args.uninstall_task,
        args.install_service,
        args.uninstall_service,
    ];
    if actions.iter().filter(|flag| **flag).count() > 1 {
        eprintln!(
            "choose only one of --install-task, --uninstall-task, --install-service, --uninstall-service"
        );
        std::process::exit(2);
    }

    if args.install_task {
        handle_install_task(args.force);
        return;
    }
    if args.uninstall_task {
        handle_uninstall_task();
        return;
    }
    if args.install_service {
        handle_install_service();
        return;
    }
    if args.uninstall_service {
        handle_uninstall_service();
        return;
    }

    let (host, port) = resolve_api_settings(args.api_host.as_deref(), args.api_port);
    let bind_addr = format!("{}:{}", host, port);

    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("Error: failed to bind API server on {} - {}", bind_addr, err);
            std::process::exit(1);
        }
    };
    let local_addr = listener
        .local_addr()
        .expect("listener should have local address");
    println!("Starting LLama Launcher API server on {}", local_addr);

    let state: SharedState = Arc::new(RwLock::new(LlamaLauncherService::new(None)));
    if let Err(err) = server::serve(listener, state).await {
        eprintln!("Error: API server failed - {}", err);
        std::process::exit(1);
    }
}
