use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use smol::channel;
use signal_hook::{self, consts::{SIGINT, SIGTERM}};
use tracing::{debug, error, info, trace};

use linux_3_finger_drag::{
    init::{config, libinput_init},
    runtime::{
        event_handler::{CancelSignal, GestureTranslator, GtError}, 
        virtual_trackpad
    }
};


fn main() -> Result<(), GtError> {

    let configs = init_cfg();
    config::init_logger(configs.clone()).init();
    trace!("Main PID: {}", std::process::id());

    // handling SIGINT and SIGTERM
    let should_exit = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGTERM, Arc::clone(&should_exit)).unwrap();
    signal_hook::flag::register(SIGINT, Arc::clone(&should_exit)).unwrap();

    let (sender, recvr) = channel::bounded::<CancelSignal>(1);
    let mut vtrackpad = virtual_trackpad::start_handler(recvr)?;

    info!("Searching for the trackpad on your device...");

    // using a match case here instead of a `?` here so the program can destruct 
    // the virtual trackpad before it exits
    let main_result = match libinput_init::find_real_trackpad() {

        Ok(mut real_trackpad) => {
            
            info!("linux-3-finger-drag started successfully!");

            let mut translator = GestureTranslator::new(
                vtrackpad.clone(), 
                configs.clone(),
                smol::Executor::new(),
                sender
            );
            
            loop {
                // this is to keep the infinite loop from filling out into
                // entire CPU core, which it will do even on no-ops.
                // This refresh rate (once per 5ms) should be sufficient 
                // for most purposes.
                std::thread::sleep(configs.response_time);

                // handle interrupts
                if should_exit.load(Ordering::Relaxed) {
                    break;
                }
                
                // Note: sometimes errors are logged by the `input` crate directly,
                // but they are non-fatal; they're typically because the system is
                // too slow to write events before their expiration. You can 
                // differentiate those by their not having a time and log-level prefix.\
                if let Err(e) = real_trackpad.dispatch() {
                    error!("A {} error occured in reading device buffer: {}", e.kind(), e);
                }

                for event in &mut real_trackpad {

                    trace!("Blocking in main()'s for loop");
                    let trans_res = smol::block_on(
                        translator.translate_gesture(event)
                    );

                    debug!("Mouse is down: {}", translator.vtp.mouse_is_down);

                    // do nothing on success (or ignored gesture)
                    if let Err(e) = trans_res { error!("{:?}", e); }
                }
            };
            Ok(())
        },
        Err(e) => Err(GtError::from(e))
    };

    // the program arrives here if either a signal is received, 
    // or there was some issue during initialization
    info!("Cleaning up and exiting...");
    vtrackpad.mouse_up()?;      // just in case
    vtrackpad.destruct()?;      // we don't need virtual devices cluttering the system
    
    info!("Clean up successful.");
    main_result
}


fn init_cfg() -> config::Configuration {
    
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

    configs
}