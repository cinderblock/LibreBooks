//! The Windows service: registration with the Service Control Manager, the
//! commands that drive it, and the entry point it starts.
//!
//! A Stop or Shutdown control is the same graceful shutdown Ctrl+C gives
//! `librebooks serve` (`ops::run_server` with a `Control` whose `stop` the
//! control handler resolves); SERVICE_STOPPED is reported only once that has
//! finished. After a self-update the service stops with exit code 75, and the
//! recovery actions registered at install start it again, as the new version.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use lb_update::RESTART_EXIT_CODE;
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceDependency, ServiceErrorControl, ServiceExitCode, ServiceFailureActions,
    ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus,
    ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use super::{DESCRIPTION, InstallArgs, Launch, ServiceCommand};
use crate::ServeArgs;
use crate::ops::Control;

/// What the SCM is told to allow for a start (opening the stores, binding).
const START_WAIT: Duration = Duration::from_secs(30);
/// …and for a stop: requests in flight, 30 s for background work, 10 s for
/// cloudflared (decision 15 in `plans/server-ops.md`).
const STOP_WAIT: Duration = Duration::from_secs(90);
/// How long `service start` / `stop` / `uninstall` wait on the SCM.
const COMMAND_WAIT: Duration = Duration::from_secs(120);

pub fn run(data_dir: &Path, secret_key: Option<PathBuf>, command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::Install(args) => install(data_dir, secret_key, args),
        ServiceCommand::Uninstall(name) => uninstall(&name.name),
        ServiceCommand::Start(name) => start(&name.name),
        ServiceCommand::Stop(name) => stop(&name.name),
        ServiceCommand::Status(name) => status(&name.name),
        ServiceCommand::Run { name, serve } => run_service(ServiceContext {
            name: name.name,
            data_dir: data_dir.to_owned(),
            secret_key,
            serve,
        }),
    }
}

/// Everything `install` registers, in one place, so `--print` shows exactly
/// what `install` would do.
#[derive(Debug, Clone)]
pub(crate) struct Registration {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub launch: Launch,
    /// `None` is LocalSystem.
    pub account: Option<String>,
}

impl Registration {
    pub fn new(name: &str, launch: Launch, account: Option<String>) -> Self {
        Registration {
            name: name.to_owned(),
            display_name: if name == super::DEFAULT_NAME {
                super::DEFAULT_NAME.to_owned()
            } else {
                format!("{} ({name})", super::DEFAULT_NAME)
            },
            description: format!(
                "{DESCRIPTION}: the API and web UI for the company files in {}.",
                launch.data_dir.display()
            ),
            launch,
            account,
        }
    }

    /// The service's command line after the program.
    pub fn arguments(&self) -> Vec<OsString> {
        self.launch.args(&["service", "run", "--name", &self.name])
    }

    fn info(&self, password: Option<OsString>) -> ServiceInfo {
        ServiceInfo {
            name: self.name.clone().into(),
            display_name: self.display_name.clone().into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: self.launch.exe.clone(),
            launch_arguments: self.arguments(),
            dependencies: vec![ServiceDependency::Service("Tcpip".into())],
            account_name: self.account.clone().map(Into::into),
            account_password: password,
        }
    }

    /// Recovery: restart after 5 s, 5 s, then 30 s; the count resets after a
    /// day without failures. With "failure actions on non-crash failures" the
    /// SCM also restarts a service that stopped itself with an error code,
    /// which is how a self-update comes back as the new version (exit 75).
    fn failure_actions() -> ServiceFailureActions {
        let restart = |secs| ServiceAction {
            action_type: ServiceActionType::Restart,
            delay: Duration::from_secs(secs),
        };
        ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![restart(5), restart(5), restart(30)]),
        }
    }

    /// What `--print` shows, and `install` confirms.
    pub fn describe(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("service name:  {}\n", self.name));
        s.push_str(&format!("display name:  {}\n", self.display_name));
        s.push_str(&format!("description:   {}\n", self.description));
        s.push_str(&format!("program:       {}\n", self.launch.exe.display()));
        s.push_str("arguments:    ");
        for a in self.arguments() {
            let a = a.to_string_lossy();
            if a.contains(' ') {
                s.push_str(&format!(" \"{a}\""));
            } else {
                s.push_str(&format!(" {a}"));
            }
        }
        s.push('\n');
        s.push_str("start:         automatic (after Tcpip)\n");
        s.push_str(&format!(
            "account:       {}\n",
            self.account.as_deref().unwrap_or("LocalSystem")
        ));
        s.push_str(
            "recovery:      restart after 5 s, 5 s, then 30 s (reset after a day), also after a\n",
        );
        s.push_str(&format!(
            "               stop with an error code (exit {RESTART_EXIT_CODE} after a self-update)\n"
        ));
        s.push_str(&format!(
            "log:           {}\n",
            self.launch.data_dir.join("logs").display()
        ));
        s
    }
}

/// A `windows_service` error with a hint for the usual causes.
fn explain(e: &windows_service::Error, what: &str) -> anyhow::Error {
    let hint = match e {
        windows_service::Error::Winapi(io) => match io.raw_os_error() {
            Some(5) => " (access denied: run this from an administrator prompt)",
            Some(1060) => " (there is no such service; `librebooks service install` registers it)",
            Some(1072) => {
                " (the service is marked for deletion; close the Services window and try again)"
            }
            Some(1073) => {
                " (a service with this name exists already; `librebooks service uninstall` first, or choose --name)"
            }
            _ => "",
        },
        _ => "",
    };
    // The crate's own Display for a Win32 failure is just "IO error in winapi
    // call"; the io::Error inside carries the system's message and code.
    let message = match e {
        windows_service::Error::Winapi(io) => io.to_string(),
        other => other.to_string(),
    };
    anyhow!("couldn't {what}: {message}{hint}")
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | access)
        .map_err(|e| explain(&e, "connect to the Service Control Manager"))
}

fn open(name: &str, access: ServiceAccess) -> Result<Service> {
    manager(ServiceManagerAccess::empty())?
        .open_service(name, access)
        .map_err(|e| explain(&e, &format!("open the service {name}")))
}

fn install(data_dir: &Path, secret_key: Option<PathBuf>, args: InstallArgs) -> Result<()> {
    let launch = Launch::new(data_dir, secret_key, args.serve)?;
    let registration = Registration::new(&args.name.name, launch, args.account);
    if args.print {
        print!("{}", registration.describe());
        println!("(nothing was registered: --print)");
        return Ok(());
    }
    let password = std::env::var_os("LIBREBOOKS_SERVICE_PASSWORD");
    if password.is_some() && registration.account.is_none() {
        bail!("LIBREBOOKS_SERVICE_PASSWORD is set but no --account was given");
    }
    let name = &registration.name;
    let service = manager(ServiceManagerAccess::CREATE_SERVICE)?
        .create_service(
            &registration.info(password),
            ServiceAccess::CHANGE_CONFIG | ServiceAccess::START,
        )
        .map_err(|e| explain(&e, &format!("register the service {name}")))?;
    service
        .set_description(&registration.description)
        .map_err(|e| explain(&e, "set the service's description"))?;
    service
        .update_failure_actions(Registration::failure_actions())
        .map_err(|e| explain(&e, "set the service's recovery actions"))?;
    service
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(|e| explain(&e, "set the service's recovery actions"))?;
    print!("{}", registration.describe());
    println!(
        "registered; start it with `librebooks service start --name {name}` (or at the next boot)"
    );
    Ok(())
}

/// Poll the service's status until `done`, or give up after `timeout`.
fn wait_for(
    service: &Service,
    done: impl Fn(ServiceState) -> bool,
    timeout: Duration,
) -> Result<ServiceStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        let status = service
            .query_status()
            .map_err(|e| explain(&e, "query the service's status"))?;
        if done(status.current_state) {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            bail!(
                "the service is still {:?} after {} s",
                status.current_state,
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn stop_and_wait(service: &Service, name: &str) -> Result<()> {
    println!("stopping {name}…");
    service
        .stop()
        .map_err(|e| explain(&e, &format!("stop the service {name}")))?;
    wait_for(service, |s| s == ServiceState::Stopped, COMMAND_WAIT)?;
    println!("{name} stopped");
    Ok(())
}

fn uninstall(name: &str) -> Result<()> {
    let service = open(
        name,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    )?;
    let status = service
        .query_status()
        .map_err(|e| explain(&e, "query the service's status"))?;
    if status.current_state != ServiceState::Stopped {
        stop_and_wait(&service, name)?;
    }
    service
        .delete()
        .map_err(|e| explain(&e, &format!("remove the service {name}")))?;
    println!("{name} is no longer a service; the data folder was left alone");
    Ok(())
}

fn start(name: &str) -> Result<()> {
    let service = open(name, ServiceAccess::QUERY_STATUS | ServiceAccess::START)?;
    let status = service
        .query_status()
        .map_err(|e| explain(&e, "query the service's status"))?;
    if status.current_state == ServiceState::Running {
        println!("{name} is already running");
        return Ok(());
    }
    service
        .start::<OsString>(&[])
        .map_err(|e| explain(&e, &format!("start the service {name}")))?;
    let status = wait_for(
        &service,
        |s| matches!(s, ServiceState::Running | ServiceState::Stopped),
        COMMAND_WAIT,
    )?;
    if status.current_state == ServiceState::Stopped {
        bail!(
            "{name} stopped right after starting ({:?}); its log is in the data folder under logs/",
            status.exit_code
        );
    }
    println!(
        "{name} is running{}",
        status
            .process_id
            .map(|pid| format!(" (pid {pid})"))
            .unwrap_or_default()
    );
    Ok(())
}

fn stop(name: &str) -> Result<()> {
    let service = open(name, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP)?;
    let status = service
        .query_status()
        .map_err(|e| explain(&e, "query the service's status"))?;
    if status.current_state == ServiceState::Stopped {
        println!("{name} is not running");
        return Ok(());
    }
    stop_and_wait(&service, name)
}

fn status(name: &str) -> Result<()> {
    let service = open(
        name,
        ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG,
    )?;
    let status = service
        .query_status()
        .map_err(|e| explain(&e, "query the service's status"))?;
    let config = service
        .query_config()
        .map_err(|e| explain(&e, "query the service's configuration"))?;
    println!(
        "{name}: {:?}{}",
        status.current_state,
        status
            .process_id
            .map(|pid| format!(" (pid {pid})"))
            .unwrap_or_default()
    );
    if status.exit_code != ServiceExitCode::NO_ERROR {
        println!("last exit:  {:?}", status.exit_code);
    }
    println!("start:      {:?}", config.start_type);
    println!("program:    {}", config.executable_path.display());
    println!(
        "account:    {}",
        config.account_name.as_deref().map_or_else(
            || "LocalSystem".to_owned(),
            |a| a.to_string_lossy().into_owned()
        )
    );
    if status.current_state != ServiceState::Running {
        // As `systemctl status` does for an inactive unit.
        std::process::exit(3);
    }
    Ok(())
}

/// What `service run` was started with, handed to `service_main` (which the
/// SCM calls back with no way to pass it anything of ours).
#[derive(Debug)]
struct ServiceContext {
    name: String,
    data_dir: PathBuf,
    secret_key: Option<PathBuf>,
    serve: ServeArgs,
}

static CONTEXT: Mutex<Option<ServiceContext>> = Mutex::new(None);

define_windows_service!(ffi_service_main, service_main);

fn run_service(ctx: ServiceContext) -> Result<()> {
    let name = ctx.name.clone();
    *CONTEXT.lock().unwrap_or_else(PoisonError::into_inner) = Some(ctx);
    // Blocks until the service has stopped.
    service_dispatcher::start(&name, ffi_service_main).map_err(|e| {
        anyhow!(
            "couldn't connect to the Service Control Manager ({e}); `service run` is what the service manager starts, not a console command: use `librebooks serve`"
        )
    })
}

/// The SCM's entry point for the service. Its arguments are those of a
/// `service start` (none); ours are the process's command line, already parsed.
fn service_main(_arguments: Vec<OsString>) {
    let Some(ctx) = CONTEXT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    else {
        return;
    };
    if let Err(e) = serve_as_service(ctx) {
        tracing::error!("the service failed: {e:#}");
    }
}

fn serve_as_service(ctx: ServiceContext) -> Result<()> {
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    // The handler runs on the SCM's thread; it gets the status handle once
    // registration has produced it.
    let handle_slot: Arc<OnceLock<ServiceStatusHandle>> = Arc::new(OnceLock::new());
    let slot = handle_slot.clone();
    let handle = service_control_handler::register(&ctx.name, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            tracing::info!(?control, "stop requested by the service manager");
            if let Some(h) = slot.get()
                && let Err(e) = report(*h, ServiceState::StopPending, ServiceExitCode::NO_ERROR)
            {
                tracing::warn!("{e}");
            }
            let _ = stop_tx.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .map_err(|e| explain(&e, "register the service's control handler"))?;
    let _ = handle_slot.set(handle);
    report(
        handle,
        ServiceState::StartPending,
        ServiceExitCode::NO_ERROR,
    )?;
    tracing::info!(
        "LibreBooks {} starting as the service {}",
        env!("CARGO_PKG_VERSION"),
        ctx.name
    );

    let stop = async move {
        let _ = stop_rx.wait_for(|s| *s).await;
    };
    let ready = move || {
        if let Err(e) = report(handle, ServiceState::Running, ServiceExitCode::NO_ERROR) {
            tracing::warn!("{e}");
        }
    };
    let outcome = crate::serve(
        &ctx.data_dir,
        &ctx.serve,
        ctx.secret_key,
        Control {
            stop: Box::pin(stop),
            ready: Box::new(ready),
            console: false,
        },
    );
    let exit_code = match &outcome {
        Ok(None) => ServiceExitCode::NO_ERROR,
        Ok(Some(_)) => {
            tracing::info!(
                "stopping with exit code {RESTART_EXIT_CODE} so the service manager starts the new version"
            );
            ServiceExitCode::ServiceSpecific(RESTART_EXIT_CODE as u32)
        }
        Err(e) => {
            tracing::error!("{e:#}");
            ServiceExitCode::ServiceSpecific(1)
        }
    };
    tracing::info!("stopped");
    report(handle, ServiceState::Stopped, exit_code)?;
    outcome.map(|_| ())
}

/// Tell the SCM where we are. Pending states carry a wait hint; Running
/// accepts Stop and Shutdown.
fn report(
    handle: ServiceStatusHandle,
    state: ServiceState,
    exit_code: ServiceExitCode,
) -> Result<()> {
    let (controls_accepted, wait_hint, checkpoint) = match state {
        ServiceState::StartPending => (ServiceControlAccept::empty(), START_WAIT, 1),
        ServiceState::StopPending => (ServiceControlAccept::empty(), STOP_WAIT, 1),
        ServiceState::Running => (
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            Duration::ZERO,
            0,
        ),
        _ => (ServiceControlAccept::empty(), Duration::ZERO, 0),
    };
    handle
        .set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted,
            exit_code,
            checkpoint,
            wait_hint,
            process_id: None,
        })
        .map_err(|e| anyhow!("couldn't report {state:?} to the service manager: {e}"))
        .context("service status")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    fn launch() -> Launch {
        let serve = match crate::Cli::try_parse_from([
            "librebooks",
            "serve",
            "--bind",
            "0.0.0.0:8740",
            "--public-url",
            "https://books.example.com",
        ])
        .expect("parses")
        .command
        {
            crate::Command::Serve { args } => args,
            other => panic!("{other:?}"),
        };
        Launch {
            exe: PathBuf::from(r"C:\Program Files\LibreBooks\librebooks.exe"),
            data_dir: PathBuf::from(r"D:\Books Data"),
            secret_key: Some(PathBuf::from(r"D:\keys\secret.key")),
            serve,
        }
    }

    #[test]
    fn registration_command_line() {
        let reg = Registration::new("LibreBooks", launch(), None);
        let args: Vec<String> = reg
            .arguments()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "service",
                "run",
                "--name",
                "LibreBooks",
                "--data-dir",
                r"D:\Books Data",
                "--secret-key-file",
                r"D:\keys\secret.key",
                "--bind",
                "0.0.0.0:8740",
                "--public-url",
                "https://books.example.com",
            ]
        );
        assert_eq!(reg.display_name, "LibreBooks");
        assert_eq!(
            Registration::new("Books2", launch(), None).display_name,
            "LibreBooks (Books2)"
        );
    }

    #[test]
    fn print_shows_what_install_registers() {
        let reg = Registration::new(
            "LibreBooks",
            launch(),
            Some(r"NT AUTHORITY\LocalService".to_owned()),
        );
        let text = reg.describe();
        assert_eq!(
            text,
            "service name:  LibreBooks\n\
             display name:  LibreBooks\n\
             description:   LibreBooks accounting server: the API and web UI for the company files in D:\\Books Data.\n\
             program:       C:\\Program Files\\LibreBooks\\librebooks.exe\n\
             arguments:     service run --name LibreBooks --data-dir \"D:\\Books Data\" --secret-key-file D:\\keys\\secret.key --bind 0.0.0.0:8740 --public-url https://books.example.com\n\
             start:         automatic (after Tcpip)\n\
             account:       NT AUTHORITY\\LocalService\n\
             recovery:      restart after 5 s, 5 s, then 30 s (reset after a day), also after a\n\
             \x20              stop with an error code (exit 75 after a self-update)\n\
             log:           D:\\Books Data\\logs\n"
        );
    }

    #[test]
    fn recovery_restarts_three_times() {
        let actions = Registration::failure_actions();
        let delays: Vec<u64> = actions
            .actions
            .expect("actions")
            .iter()
            .map(|a| {
                assert_eq!(a.action_type, ServiceActionType::Restart);
                a.delay.as_secs()
            })
            .collect();
        assert_eq!(delays, [5, 5, 30]);
    }
}
