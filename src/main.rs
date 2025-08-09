use std::{sync::{atomic::{AtomicBool, Ordering}, Arc}, time::Duration};
use smol::channel::{self, Sender};
use signal_hook::{self, consts::{SIGINT, SIGTERM}, flag};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, trace};

use linux_3_finger_drag::{
    init::{config::{self, Configuration}, libinput_init},
    runtime::{
        event_handling::{ControlSignal, GestureTranslator, GtError, TranslatorClassic, TranslatorCd}, 
        virtual_trackpad::{self, VirtualTrackpad}
    }
};


#[tokio::main]
async fn main() -> Result<(), GtError> {

    let configs = config::init_cfg();
    config::init_logger(configs.clone()).init();

    // handling SIGINT and SIGTERM
    let should_exit = Arc::new(AtomicBool::new(false));
    flag::register(SIGTERM, Arc::clone(&should_exit)).unwrap();
    flag::register(SIGINT,  Arc::clone(&should_exit)).unwrap();

    let (sender, recvr) = channel::bounded::<ControlSignal>(3);
    let mut vtrackpad = virtual_trackpad::start_handler(recvr)?;

    info!("Searching for the trackpad on your device...");

    // using a match case here instead of a `?` here so the program can destruct 
    // the virtual trackpad before it exits
    let mut mouse_up_listener = None;
    let main_result = match libinput_init::find_real_trackpad() {

        Ok(real_trackpad) => {

            let (mouse_up_listener, translator ) = determine_fork(&vtrackpad, configs, sender);
            
            run_main_event_loop(&should_exit, real_trackpad, translator).await?;

            // join the other thread, if it was made
            if let Some(handle) = mouse_up_listener {
                trace!("Joining delay timer thread");
                translator.send_signal(ControlSignal::TerminateThread).await.unwrap();
                handle.await?
            } else {
                Ok(())
            }
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


fn determine_fork(
    vtrackpad: &VirtualTrackpad, 
    configs: Configuration, 
    sender: Sender<ControlSignal>
) -> ( Option<JoinHandle<Result<(), GtError>>>, impl GestureTranslator ) {

    // spawn 1 separate thread to handle mouse_up_delay timeouts
    if configs.drag_end_delay != Duration::ZERO 
    && configs.drag_end_delay_cancellable {
        
        debug!("Creating new thread to manage drag end timer");
        let mut vtp_clone = vtrackpad.clone();
        let delay = configs.drag_end_delay;

        let fork_fn = async move {
            vtp_clone.handle_mouse_up_timeout(delay)
                .await
                .map_err(GtError::from)
        };

        let translator = TranslatorMt {
            vtp: vtrackpad.clone(), 
            cfg: configs.clone(),
            tx: sender
        };

        return (Some(tokio::spawn(fork_fn)), translator);
    }

    let translator = TranslatorClassic {
        vtp: vtrackpad.clone(), 
        cfg: configs.clone(),
    };

    (None, translator)
}

// This function is placed in `main.rs` since it's essentially a 
// part of `main`, and I wanted to break it out so the `main` isn't
// too sprawling
async fn run_main_event_loop(
    should_exit: &Arc<AtomicBool>,
    mut real_trackpad: input::Libinput, 
    mut translator: impl GestureTranslator
) -> Result<(), GtError> {

    info!("linux-3-finger-drag started successfully!");

    loop {
        // this is to keep the infinite loop from filling out into
        // entire CPU core, which it will do even on no-ops.
        std::thread::sleep(translator.response_time());

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
        }
    };

    Ok(())
}