use std::sync::{Arc, RwLock};

use clap::Parser;
use llama_launcher::server;
use llama_launcher::server::SharedState;
use llama_launcher::service::LlamaLauncherService;
use llama_launcher::service_install::{
    install_service, install_task, uninstall_service, uninstall_task, SERVICE_NAME,
};
use tokio::net::TcpListener;

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::sync::Mutex;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use windows_service::define_windows_service;
#[cfg(windows)]
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
#[cfg(windows)]
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
#[cfg(windows)]
use windows_service::service_dispatcher;

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

#[cfg(windows)]
define_windows_service!(ffi_service_main, windows_service_main);

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

fn apply_startup_profile_on_boot(service: &LlamaLauncherService) {
    if let Err(err) = service.apply_startup_profile() {
        eprintln!("Warning: failed to auto-apply startup profile: {}", err);
    }
}

fn restore_versions_on_boot(service: &LlamaLauncherService) {
    let restored = service.restore_installed_versions_from_disk();
    if restored > 0 {
        println!("Restored {} installed runtime(s) from disk", restored);
    }
}

async fn run_api_server(host: String, port: i64) -> Result<(), String> {
    let bind_addr = format!("{}:{}", host, port);

    let listener = TcpListener::bind(&bind_addr)
        .await
        .map_err(|err| format!("failed to bind API server on {} - {}", bind_addr, err))?;
    let local_addr = listener
        .local_addr()
        .expect("listener should have local address");
    println!("Starting LLama Launcher API server on {}", local_addr);

    let service = LlamaLauncherService::new(None);
    restore_versions_on_boot(&service);
    apply_startup_profile_on_boot(&service);
    let state: SharedState = Arc::new(RwLock::new(service));
    server::serve(listener, state)
        .await
        .map_err(|err| format!("API server failed - {}", err))
}

#[cfg(windows)]
fn try_run_as_windows_service() -> Result<bool, String> {
    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(()) => Ok(true),
        Err(windows_service::Error::Winapi(err))
            if err.raw_os_error() == Some(1063) =>
        {
            Ok(false)
        }
        Err(err) => Err(format!(
            "failed to connect to Windows Service Control Manager: {}",
            err
        )),
    }
}

#[cfg(not(windows))]
fn try_run_as_windows_service() -> Result<bool, String> {
    Ok(false)
}

#[cfg(windows)]
fn windows_service_main(arguments: Vec<OsString>) {
    let _ = run_windows_service(arguments);
}

#[cfg(windows)]
fn run_windows_service(arguments: Vec<OsString>) -> Result<(), String> {
    let shutdown_tx = Arc::new(Mutex::new(None::<tokio::sync::oneshot::Sender<()>>));
    let shutdown_tx_for_handler = Arc::clone(&shutdown_tx);
    let status_handle = service_control_handler::register(SERVICE_NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Some(tx) = shutdown_tx_for_handler
                .lock()
                .expect("service shutdown lock poisoned")
                .take()
            {
                let _ = tx.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .map_err(|err| format!("failed to register service control handler: {}", err))?;

    let start_pending = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 1,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    };
    status_handle
        .set_service_status(start_pending)
        .map_err(|err| format!("failed to report service start pending status: {}", err))?;

    let mut cli_args = vec![OsString::from("llama-launcher-service")];
    // The SCM passes the service name as first argument (e.g. "LlamaLauncher").
    // Skip it, then forward only actual CLI options.
    let mut service_args = arguments.into_iter();
    let _ = service_args.next();
    cli_args.extend(service_args);
    let cli = Cli::try_parse_from(cli_args).map_err(|err| err.to_string())?;
    let (host, port) = resolve_api_settings(cli.api_host.as_deref(), cli.api_port);

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|err| format!("failed to create Tokio runtime for Windows service: {}", err))?;
    let bind_addr = format!("{}:{}", host, port);
    let listener = runtime.block_on(async {
        let mut last_err = None;
        for attempt in 1..=30 {
            match TcpListener::bind(&bind_addr).await {
                Ok(listener) => return Ok(listener),
                Err(err) => {
                    last_err = Some(err);
                    if attempt < 30 {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
        Err(last_err.expect("bind retry loop should capture last error"))
    })
    .map_err(|err| format!("failed to bind API server on {} after retry - {}", bind_addr, err))?;

    let local_addr = listener
        .local_addr()
        .expect("service listener should have local address");
    println!("Starting LLama Launcher API server on {}", local_addr);

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    *shutdown_tx
        .lock()
        .expect("service shutdown lock poisoned") = Some(stop_tx);

    let running = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    status_handle
        .set_service_status(running)
        .map_err(|err| format!("failed to report service running status: {}", err))?;

    let service = LlamaLauncherService::new(None);
    restore_versions_on_boot(&service);
    apply_startup_profile_on_boot(&service);
    let state: SharedState = Arc::new(RwLock::new(service));
    let service_for_shutdown = Arc::clone(&state);
    let serve_result = runtime.block_on(async move {
        server::serve_with_shutdown(listener, state, async move {
            let _ = stop_rx.await;
            let service = service_for_shutdown
                .read()
                .expect("service lock poisoned during shutdown");
            service.stop();
        })
        .await
    });

    let stop_pending = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StopPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 1,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    };
    let _ = status_handle.set_service_status(stop_pending);

    let exit_code = if serve_result.is_ok() { 0 } else { 1 };
    let stopped = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(exit_code),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    let _ = status_handle.set_service_status(stopped);

    serve_result.map_err(|err| format!("API server failed - {}", err))
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

    match try_run_as_windows_service() {
        Ok(true) => return,
        Ok(false) => {}
        Err(err) => {
            eprintln!("Error: {}", err);
            std::process::exit(1);
        }
    }

    let (host, port) = resolve_api_settings(args.api_host.as_deref(), args.api_port);
    if let Err(err) = run_api_server(host, port).await {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }
}
