use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use signal_hook::{self, consts::{SIGINT, SIGTERM}};
#[macro_use] extern crate log;

mod config;
mod event_handler;
mod libinput_init;
mod virtual_trackpad;

fn main() -> Result<(), std::io::Error> {

    println!("[PRE-LOG: INFO]: Loading configuration...");
    let configs = match config::parse_config_file() {
        Ok(cfg) => {
            println!("[PRE-LOG: INFO]: Successfully loaded your configuration (with defaults for unspecified values): \n{:#?}", &cfg);
            cfg
        },
        Err(err) => {
            let cfg = Default::default();
            println!("\n[PRE-LOG: WARNING]: {err}\n\nThe configuration file could not be \
                loaded, so the program will continue with defaults of:\n{cfg:#?}",
            );
            cfg
        }
    };

    if let Err(_) = config::init_logger(configs.clone()) {
        // the only error that gets raised is a SetLoggerError,
        // which only occurs when a logger has already been set
        // for the program (not really possible in this program,
        // but trying to cover all the bases here)
        println!(
            "[PRE-LOG: WARNING]: a logger seems to have already been \
            initialized for this program."
        );
    };

    let fail_fast = configs.fail_fast;

    // handling SIGINT and SIGTERM
    let should_exit = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGTERM, Arc::clone(&should_exit)).unwrap();
    signal_hook::flag::register(SIGINT, Arc::clone(&should_exit)).unwrap();


    let mut vtrackpad = virtual_trackpad::start_handler()?;

    info!("Searching for the trackpad on your device...");

    let main_result = match libinput_init::find_real_trackpad() {

        Ok(mut real_trackpad) => {
            
            info!("linux-3-finger-drag started successfully!");

            // `latest_runtime_error` gets set to latest runtime error, 
            // if any occur.
            //
            // The program only exits during runtime when terminated 
            // via signal from the OS, but if errors occurred, it 
            // will exit with a non-zero status.
            let mut latest_runtime_error: Result<(), std::io::Error> = Ok(());  
            loop {

                // handle interrupts
                if should_exit.load(Ordering::Relaxed) {
                    break;
                }
                
                // I am aware that the use of println!() in this loop is inefficient, 
                // but the errors they are logging should be very rare and should
                // therefore not incur too high a burden on the CPU.
                //
                // Note: sometimes errors are logged by the `input` crate directly,
                // but they are non-fatal; they're typically because the system is
                // too slow to write events before their expiration. You can 
                // differentiate those by their not having a time and log-level prefix.
                if let Err(e) = real_trackpad.dispatch() {
                    error!("A {} error occured during runtime: {}", e.kind(), e);
                }

                for event in &mut real_trackpad {
                    
                    //if !matches!(event, Event::Gesture(_)) { continue; }

                    // do nothing on success (or ignored gesture)
                    if let Err(e) = event_handler::translate_gesture(event, &mut vtrackpad, &configs) {

                        error!("A {} error occured during runtime: {}", e.kind(), e);

                        if fail_fast { return Err(e); }
                        
                        latest_runtime_error = Err(e);  // update exit status to latest runtime error
                    };
                }
            }

            latest_runtime_error
        },
        Err(e) => Err(e)
    };

    // the program arrives here if either a signal is received, 
    // or there was some issue during initialization
    info!("Cleaning up and exiting...");
    vtrackpad.mouse_up()?;      // just in case
    vtrackpad.destruct()?;      // we don't need virtual devices cluttering the system
    
    info!("Clean up successful.");
    main_result
}
