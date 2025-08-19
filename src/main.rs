use std::{sync::{atomic::{AtomicBool, Ordering}, Arc}};
use tokio::sync::mpsc::{self, Receiver};
use signal_hook::{self, consts::{SIGINT, SIGTERM}, flag};
use tracing::{debug, error, info, trace};
use tracing_subscriber::fmt::time::ChronoLocal;

use linux_3_finger_drag::{
    init::{config, libinput_init},
    runtime::{
        event_handler::{ControlSignal, GestureTranslator, GtError}, 
        virtual_trackpad
    }
};


#[tokio::main]
async fn main() -> Result<(), GtError> {

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
    println!("[PRE-LOG: INFO]: Logger initialized!"); 

    // handling SIGINT and SIGTERM
    let should_exit = Arc::new(AtomicBool::new(false));
    flag::register(SIGTERM, Arc::clone(&should_exit)).unwrap();
    flag::register(SIGINT,  Arc::clone(&should_exit)).unwrap();

    let (sender, recvr) = mpsc::channel::<ControlSignal>(3);
    let mut vtrackpad = virtual_trackpad::start_handler()?;

    info!("Searching for the trackpad on your device...");

    // using a match case here instead of a `?` here so the program can destruct 
    // the virtual trackpad before it exits
    let main_result = match libinput_init::find_real_trackpads() {

        Ok(real_trackpad) => {

            let translator = GestureTranslator::new(
                vtrackpad.clone(), 
                configs.clone(),
                sender
            );
            run_main_event_loop(translator, recvr, &should_exit, real_trackpad).await
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


// This function is placed in `main.rs` since it's essentially a 
// part of `main`, and I wanted to break it out so the `main` isn't
// too sprawling
async fn run_main_event_loop(
    mut translator: GestureTranslator,
    recvr: Receiver<ControlSignal>,
    should_exit: &Arc<AtomicBool>,
    mut real_trackpad: input::Libinput, 
    
) -> Result<(), GtError> {

    // spawn 1 separate thread to handle mouse_up_delay timeouts
    debug!("Creating new thread to manage drag end timer");
    let mut vtp_clone = translator.vtp.clone();
    let delay = translator.cfg.drag_end_delay;

    let fork_fn = async move {
        vtp_clone.handle_mouse_up_timeout(delay, recvr)
            .await
            .map_err(GtError::from)
    };

    let mouse_up_listener = tokio::spawn(fork_fn);


    info!("linux-3-finger-drag started successfully!");

    loop {
        // this is to keep the infinite loop from filling out into
        // entire CPU core, which it will do even on no-ops.
        std::thread::sleep(translator.cfg.response_time);

        // handle interrupts
        if should_exit.load(Ordering::Relaxed) {
            break;
        }
        
        if let Err(e) = real_trackpad.dispatch() {
            error!("A {} error occured in reading device buffer: {}", e.kind(), e);
        }

        for event in &mut real_trackpad {

            trace!("Blocking in main()'s for loop");

            // do nothing on success (or ignored gesture)
            if let Err(e) = translator.translate_gesture(event).await { 
                error!("{:?}", e); 
            }

            // Without being a `ControlSignal::TerminateThread` being sent
            // into the channel, the other thread only finishes when
            // is an error is raised. it has been designed not to panic. 
            // the value the thread returns is a `Result`, so the this extracts 
            // the Result from the fork and returns it.
            if mouse_up_listener.is_finished() {
                let fork_err = mouse_up_listener.await?.unwrap_err();
                error!("Error raised in fork: {:?}", fork_err);
                return Err(fork_err);
            }
        }
    };

    debug!("Joining delay timer thread");
    translator.send_signal(ControlSignal::TerminateThread).await?;

    // awaiting a JoinHandle produces a Result
    // the generic for this JoinHandle, though, is itself a Result, 
    // so we can just return what the JoinHandle yields
    mouse_up_listener.await? 
}