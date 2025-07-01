use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use signal_hook::{self, consts::{SIGINT, SIGTERM}};

mod virtual_trackpad;
mod event_handler;
mod init_fns;


fn main() -> Result<(), std::io::Error> {

    let configs = match init_fns::config::parse_config_file() {
        Ok(cfg) => cfg,
        Err(err) => {
            let cfg = Default::default();
            println!("WARNING: {err}\n\nThe configuration file (at least) will not be accessed, \
                and the program will continue execution (if possible), using defaults of:\n {cfg:#?}");
            cfg
        }
    };

    let mut vtrackpad = virtual_trackpad::start_handler();

    // handling SIGINT and SIGTERM
    let should_exit = Arc::new(AtomicBool::new(false));

    signal_hook::flag::register(SIGTERM, Arc::clone(&should_exit)).unwrap();
    signal_hook::flag::register(SIGINT, Arc::clone(&should_exit)).unwrap();

    
    let mut real_trackpad = init_fns::libinput_init::find_real_trackpad()?;

    loop {

        // handle interrupts
        if should_exit.load(Ordering::Relaxed) {
            break;
        }
        
        if let Err(e) = real_trackpad.dispatch() {
            println!("ERROR during runtime: {:?}", e);
        }

        for event in &mut real_trackpad {
            
            event_handler::translate_gesture(event, &mut vtrackpad, &configs);
        }
    }

    println!("\nSignal received, cleaning up and exiting...");
    vtrackpad.mouse_up();    // just in case
    vtrackpad.dev_destroy(); // we don't need virtual devices cluttering the system

    Ok(())
}
