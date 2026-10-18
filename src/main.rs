mod admin;
mod app;
mod bootstrap;
mod cli;
mod gateway;
mod offline;
mod routes;
mod server;
mod telemetry;
mod web;

use std::process::ExitCode;

use bootstrap::{Loaded, ServeResources};
use cli::{Cli, Command};

/// The application owns process exit; the VM library only reports retirement.
fn main() -> ExitCode {
    telemetry::install_panic_hook();
    let cli = Cli::parse();
    let (path, serve) = match &cli.command {
        Command::Check(args) => (&args.config, false),
        Command::Serve(args) => (&args.config, true),
        Command::Test(args) => (&args.configuration.config, false),
    };
    let executor = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(executor) => executor,
        Err(source) => {
            bootstrap::report_failure(&bootstrap::Error::Executor(source), "startup", "executor");
            return ExitCode::FAILURE;
        },
    };
    let result = if let Command::Test(args) = &cli.command {
        executor.block_on(offline::run(path.as_deref(), &args.cases))
    } else {
        executor.block_on(run(path.as_deref(), serve))
    };
    // All component shutdowns have their own budgets. A stuck filesystem syscall
    // must not turn dropping the executor into an unbounded process shutdown.
    executor.shutdown_timeout(std::time::Duration::ZERO);
    result
}

async fn run(path: Option<&std::path::Path>, serve: bool) -> ExitCode {
    if !serve {
        let loaded = match Loaded::load(path).await {
            Ok(loaded) => loaded,
            Err(error) => {
                bootstrap::report_failure(&error, "check", error.phase());
                return ExitCode::FAILURE;
            },
        };
        if let Err(error) = loaded.shutdown().await {
            bootstrap::report_failure(&error, "check", error.phase());
            return ExitCode::FAILURE;
        }
        println!(
            "Configuration valid ({} providers, {} models).",
            loaded.business.providers.len(),
            loaded.business.models.len()
        );
        return ExitCode::SUCCESS;
    }
    let resources = match ServeResources::load(path).await {
        Ok(resources) => resources,
        Err(error) => {
            bootstrap::report_failure(&error, "serve", error.phase());
            return ExitCode::FAILURE;
        },
    };
    let shutdown = limen_script::Cancellation::new();
    let primary = match application(&resources, shutdown.clone()) {
        Ok(application) => server::serve(
            http_settings(&resources.loaded.server),
            application,
            shutdown,
            resources.loaded.runtime.status_changes(),
        )
        .await
        .map_err(bootstrap::Error::from),
        Err(error) => Err(error),
    };
    let result = match (primary, resources.shutdown().await) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(primary), Err(cleanup)) => Err(bootstrap::Error::Cleanup {
            primary: Box::new(primary),
            cleanup: Box::new(cleanup),
        }),
    };
    if let Err(error) = result {
        bootstrap::report_failure(&error, "serve", error.phase());
        if !resources.logging.shutdown() {
            bootstrap::report_failure(&bootstrap::Error::LogShutdown, "serve", "shutdown");
        }
        return ExitCode::FAILURE;
    }
    if !resources.logging.shutdown() {
        bootstrap::report_failure(&bootstrap::Error::LogShutdown, "serve", "shutdown");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn http_settings(server: &limen_config::ServerConfig) -> server::HttpSettings {
    let header_timeout = std::time::Duration::from_millis(server.limits.request_ms);
    server::HttpSettings {
        listen: server.listen,
        connection_limit: server.limits.concurrency + server.limits.queue_capacity,
        header_timeout,
        transport_budget: header_timeout
            + std::time::Duration::from_millis(server.limits.cleanup_ms),
        shutdown_timeout: std::time::Duration::from_millis(server.limits.shutdown_ms),
    }
}

fn application(
    resources: &ServeResources,
    shutdown: limen_script::Cancellation,
) -> Result<std::sync::Arc<app::Application>, bootstrap::Error> {
    use std::{sync::Arc, time::Duration};
    let loaded = &resources.loaded;
    let server = &loaded.server;
    let transport = gateway::Transport::http(
        loaded
            .business
            .providers
            .values()
            .map(|provider| gateway::safe_target(&provider.endpoint))
            .collect(),
    )
    .map_err(bootstrap::Error::UpstreamClient)?;
    let generation = gateway::Gateway::new(
        gateway::GenerationConfig {
            limits: server.limits.clone(),
            client_headers: server.script_context.client_headers.clone(),
            observation_names: server.observability.events.clone(),
            storage_operation_ms: server.storage.operation_ms,
            business: Arc::clone(&loaded.business),
            plans: Arc::clone(&loaded.plans),
        },
        gateway::GenerationServices {
            runtime: loaded.runtime.clone(),
            transport,
            records: gateway::RecordSink::Persistent(resources.storage.writer()),
            shutdown,
            run_id: resources.run_id.clone(),
        },
    );
    let admin = server.auth.manager_key.as_ref().map(|_| {
        admin::Admin::new(
            admin::ManagementSettings {
                concurrency: server.management.concurrency,
                timeout: Duration::from_millis(server.management.timeout_ms),
            },
            admin::AdminConfigSnapshot::new(server, &loaded.business),
            admin::AdminServices {
                run_id: resources.run_id.clone(),
                started_at: resources.started_at.clone(),
                runtime_status: loaded.runtime.status_changes(),
                storage: resources.storage.reader(),
                logging: resources.logging.health(),
                logs: resources.logging.reader(),
            },
        )
    });
    Ok(app::Application::new(
        server.auth.clone(),
        gateway::models_json(&loaded.business),
        app::ApplicationSettings {
            body_timeout: Duration::from_millis(server.limits.request_ms),
            max_output_bytes: server.limits.max_output_bytes,
        },
        resources.run_id.clone(),
        generation,
        admin,
    ))
}
