use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

use tokio::io::unix::AsyncFd;
use tokio::io::Interest;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, info, warn};
use tracing_subscriber::fmt::time::ChronoLocal;

use linux_3_finger_drag::{
    init::{config, libinput_init},
    runtime::{gesture::GestureMachine, mt_proxy::MtProxy, virtual_trackpad},
};

/// How often the config file's mtime is checked for hot reload.
const CFG_POLL: Duration = Duration::from_secs(2);
/// Hotplug: how long to keep retrying discovery after the touchpad
/// disappears (device re-enumeration, e.g. around suspend), before
/// giving up and letting the service manager restart us.
const REDISCOVER_ATTEMPTS: u32 = 60;
const REDISCOVER_BACKOFF: Duration = Duration::from_millis(500);

struct Args {
    /// Explicit touchpad device path (skips discovery). Mainly for the
    /// integration test harness, but also useful on multi-touchpad
    /// machines.
    device: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { device: None };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--device" => {
                args.device = Some(
                    iter.next()
                        .ok_or_else(|| "--device requires a path argument".to_string())?,
                );
            }
            "--version" | "-V" => {
                println!("linux-3-finger-drag {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "linux-3-finger-drag [--device /dev/input/eventN]\n\n\
                    Turns a sustained 3-finger touchpad touch into a drag \
                    (mouse-button-held movement).\n\n\
                      --device PATH   proxy this evdev device instead of \
                    auto-discovering the touchpad\n\
                      --version       print the version and exit"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unrecognized argument: {other}")),
        }
    }
    Ok(args)
}

/// Wraps just the raw fd for readiness-polling; the proxy keeps
/// ownership of the actual File.
struct FdWatch(RawFd);
impl AsRawFd for FdWatch {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

/// Sleep until `deadline`, or forever if there is none. Used as a
/// select! arm so gesture-decision deadlines fire exactly on time.
async fn sleep_until_opt(deadline: Option<std::time::Instant>) {
    match deadline {
        Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t)).await,
        None => std::future::pending::<()>().await,
    }
}

fn is_unplug(err: &io::Error) -> bool {
    err.raw_os_error() == Some(libc::ENODEV)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), io::Error> {
    let args = parse_args().map_err(|e| {
        eprintln!("{e}\nTry --help.");
        io::Error::new(io::ErrorKind::InvalidInput, e)
    })?;

    let configs = config::init_cfg();

    match config::init_file_logger(configs.clone()) {
        Some(logger) => logger.init(),
        None => {
            tracing_subscriber::fmt()
                .with_writer(std::io::stdout)
                .with_max_level(configs.log_level)
                .with_timer(ChronoLocal::rfc_3339())
                .init();
        }
    };

    let mut vtrackpad = virtual_trackpad::start_handler()?;

    // run() holds the real event loop; wrapping it like this guarantees
    // the virtual devices are destroyed on the way out no matter how it
    // returns (including the button being released if a drag was live).
    let result = run(&args, configs, &mut vtrackpad).await;

    info!("Cleaning up and exiting...");
    vtrackpad.mouse_up()?; // just in case a drag was in flight
    vtrackpad.destruct()?;
    info!("Clean up successful.");
    result
}

async fn run(
    args: &Args,
    mut cfg: config::Configuration,
    vtp: &mut virtual_trackpad::VirtualTrackpad,
) -> Result<(), io::Error> {
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    let cfg_path = config::get_config_file_path()?;
    let mut cfg_mtime = std::fs::metadata(&cfg_path).and_then(|m| m.modified()).ok();
    let mut cfg_timer = tokio::time::interval(CFG_POLL);
    cfg_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Outer loop: one iteration per (re)acquired touchpad. Re-entered
    // only if the device disappears (ENODEV) and rediscovery succeeds.
    'device: loop {
        let path = match &args.device {
            Some(p) => p.clone(),
            None => {
                info!("Searching for the trackpad on your device...");
                let paths = libinput_init::find_real_trackpads()?;
                if paths.len() > 1 {
                    warn!(
                        "Found {} touchpads; only proxying the first one ({}).",
                        paths.len(),
                        paths[0]
                    );
                }
                paths[0].clone()
            }
        };

        let mut proxy = MtProxy::new(&path)?;
        let mut machine = GestureMachine::new(
            cfg.timing(),
            proxy.x_res(),
            proxy.y_res(),
            proxy.slot_count(),
        );
        let watch = AsyncFd::with_interest(FdWatch(proxy.as_raw_fd()), Interest::READABLE)?;

        info!("linux-3-finger-drag started successfully!");

        // Inner loop: fully event-driven. We wake for exactly three
        // reasons: the touchpad has events, a gesture decision deadline
        // arrived, or housekeeping (config reload / shutdown signal).
        let lost_device = loop {
            tokio::select! {
                ready = watch.readable() => {
                    let mut guard = ready?;
                    match proxy.drain(&mut machine, vtp) {
                        Ok(()) => { guard.clear_ready(); }
                        Err(e) if is_unplug(&e) => break true,
                        Err(e) => return Err(e),
                    }
                }

                _ = sleep_until_opt(machine.next_deadline()) => {
                    let outs = machine.on_tick(std::time::Instant::now());
                    proxy.apply(&outs, vtp)?;
                }

                _ = cfg_timer.tick() => {
                    let new_mtime = std::fs::metadata(&cfg_path)
                        .and_then(|m| m.modified())
                        .ok();
                    if new_mtime.is_some() && new_mtime != cfg_mtime {
                        cfg_mtime = new_mtime;
                        cfg = config::init_cfg();
                        machine.set_timing(cfg.timing());
                        info!("Configuration reloaded (log settings need a restart).");
                    }
                }

                _ = sigterm.recv() => break false,
                _ = sigint.recv() => break false,
            }
        };

        if !lost_device {
            proxy.destruct()?;
            return Ok(());
        }

        // The touchpad vanished (re-enumeration / suspend quirk). Drop
        // the dead handles, release the button if a drag was mid-flight,
        // and try to find it again -- the systemd unit's Restart is the
        // backstop if it never comes back.
        warn!("Touchpad disappeared (ENODEV); attempting rediscovery...");
        if machine.button_held() {
            vtp.mouse_up()?;
        }
        let _ = proxy.destruct();

        if args.device.is_some() {
            // an explicitly given device won't be re-discovered; bail
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "explicitly specified device disappeared",
            ));
        }

        for attempt in 1..=REDISCOVER_ATTEMPTS {
            tokio::time::sleep(REDISCOVER_BACKOFF).await;
            match libinput_init::find_real_trackpads() {
                Ok(paths) if !paths.is_empty() => {
                    debug!("Touchpad back after {attempt} attempt(s).");
                    continue 'device;
                }
                _ => {}
            }
        }
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "touchpad did not reappear after re-enumeration",
        ));
    }
}
