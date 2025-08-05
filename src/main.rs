use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use smol::channel::unbounded;
use signal_hook::{self, consts::{SIGINT, SIGTERM}};
#[macro_use] extern crate log;

use linux_3_finger_drag::{
    init::{config, libinput_init},
    runtime::{
        event_handler::{CancelMouseUpDelay, GestureTranslator, GtError}, 
        virtual_trackpad
    }
};


fn main() -> Result<(), GtError> {

    let configs = init_cfg();

    // handling SIGINT and SIGTERM
    let should_exit = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGTERM, Arc::clone(&should_exit)).unwrap();
    signal_hook::flag::register(SIGINT, Arc::clone(&should_exit)).unwrap();

    let (sender, recvr) = unbounded::<CancelMouseUpDelay>();
    let mut vtrackpad = virtual_trackpad::start_handler(recvr)?;

    info!("Searching for the trackpad on your device...");

    // using a match case here instead of a `?` here so the program can destruct 
    // the virtual trackpad before it exits
    let main_result = match libinput_init::find_real_trackpad() {

        Ok(mut real_trackpad) => {
            
            info!("linux-3-finger-drag started successfully!");

            // lightweight async runtime, so you don't have to compile tokio. 
            // I'm using two runtimes so the async tasks are scoped, which helps
            // to make sure any running forks get properly cleaned up dropped 
            // when the program exits.
            let gt_async_rt = smol::Executor::new();
            let main_async_rt = smol::Executor::new();

            let mut translator = GestureTranslator::new(
                vtrackpad.clone(), 
                configs.clone(), 
                gt_async_rt, 
                sender
            );
            
            loop {

                //debug!("starting loop...");
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
                // differentiate those by their not having a time and log-level prefix.
                if let Err(e) = real_trackpad.dispatch() {
                    error!("A {} error occured in reading device buffer: {}", e.kind(), e);
                }

                for event in &mut real_trackpad {

                    debug!("Blocking in main()");
                    let trans_res = smol::block_on(
                        main_async_rt.run(
                            translator.translate_gesture(event)
                        )
                    );

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

    configs
}