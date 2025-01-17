use clap::Parser;
use dozer_orchestrator::cli::generate_config_repl;
use dozer_orchestrator::cli::types::{ApiCommands, AppCommands, Cli, Commands, ConnectorCommands};
use dozer_orchestrator::cli::{init_dozer, list_sources, LOGO};
use dozer_orchestrator::errors::{CliError, OrchestrationError};
use dozer_orchestrator::simple::SimpleOrchestrator;
use dozer_orchestrator::{set_ctrl_handler, set_panic_hook, Orchestrator};
use dozer_types::tracing::{error, info};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{process, thread};

fn main() {
    set_panic_hook();

    if let Err(e) = run() {
        error!("{}", e);
        process::exit(1);
    }
}

fn render_logo() {
    use std::println as info;
    const VERSION: &str = env!("CARGO_PKG_VERSION");

    info!("{LOGO}");
    info!("\nDozer Version: {VERSION}\n");
}

fn run() -> Result<(), OrchestrationError> {
    // Reloading trace layer seems impossible, so we are running Cli::parse in a closure
    // and then initializing it after reading the configuration. This is a hacky workaround, but it works.

    let cli = parse_and_generate()?;
    let mut dozer = init_orchestrator(&cli)?;

    let running = Arc::new(AtomicBool::new(true));
    set_ctrl_handler(running.clone());

    let tel_running = running.clone();

    // Now we have acces to telemetry configuration
    let telemetry_config = dozer.config.telemetry.clone();

    // start tracing in a different thread as it needs a tokio runtime.

    let _tracing_thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("cannot start runtime");
        runtime.block_on(async {
            dozer_tracing::init_telemetry(None, telemetry_config);

            // Keep thread running until the main thread is running
            while tel_running.load(std::sync::atomic::Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }

            dozer_tracing::shutdown_telemetry();
        });
    });

    if let Some(cmd) = cli.cmd {
        // run individual servers
        match cmd {
            Commands::Api(api) => match api.command {
                ApiCommands::Run => {
                    render_logo();
                    let running_api = running.clone();
                    let _api_thread = thread::spawn(move || {
                        if let Err(e) = dozer.run_api(running_api) {
                            std::panic::panic_any(e);
                        }
                    });
                    while running.load(Ordering::SeqCst) {}
                    Ok(())
                }
                ApiCommands::GenerateToken => {
                    let token = dozer.generate_token()?;
                    info!("token: {:?} ", token);
                    Ok(())
                }
            },
            Commands::App(apps) => match apps.command {
                AppCommands::Run => {
                    render_logo();

                    dozer.run_apps(running, None)
                }
            },
            Commands::Connector(sources) => match sources.command {
                ConnectorCommands::Ls => list_sources(&cli.config_path),
            },
            Commands::Migrate(migrate) => {
                let force = migrate.force.is_some();

                dozer.migrate(force)
            }
            Commands::Clean => dozer.clean(),
            Commands::Init => {
                panic!("This should not happen as it is handled in parse_and_generate");
            }
        }
    } else {
        render_logo();

        let mut dozer = init_dozer(cli.config_path)?;
        dozer.run_all(running)
    }
}

// Some commands dont need to initialize the orchestrator
// This function is used to run those commands
fn parse_and_generate() -> Result<Cli, OrchestrationError> {
    dozer_tracing::init_telemetry_closure(None, None, || -> Result<Cli, OrchestrationError> {
        let cli = Cli::parse();

        if let Some(Commands::Init) = cli.cmd {
            if let Err(e) = generate_config_repl() {
                error!("{}", e);
                Err(e)
            } else {
                // We need to exit here, otherwise the orchestrator will be initialized
                process::exit(0);
            }
        } else {
            Ok(cli)
        }
    })
}

fn init_orchestrator(cli: &Cli) -> Result<SimpleOrchestrator, CliError> {
    dozer_tracing::init_telemetry_closure(None, None, || -> Result<SimpleOrchestrator, CliError> {
        let res = init_dozer(cli.config_path.clone());

        match res {
            Ok(dozer) => Ok(dozer),
            Err(e) => {
                error!("{}", e);
                Err(e)
            }
        }
    })
}
