use smol::channel::{RecvError, SendError, Sender};
use input::{
    Event,
    event::{
        gesture::{
            GestureEvent, 
            GestureEventCoordinates,
            GestureEventTrait,
            GestureHoldEvent,
            GestureSwipeEvent,
            GestureSwipeUpdateEvent
        },
        pointer::PointerEvent
    }
};


use log::debug;

use super::virtual_trackpad::{VirtualTrackpad, VtpError};
use super::super::init::config::Configuration;

// the "signal" to send into channel to cancel 
pub struct CancelMouseUpDelay;

// (G)esture (T)ranslation Error
#[derive(Debug)]
pub enum GtError {
    EventWriteError(std::io::Error),
    ChildThreadPanicked(std::io::Error),
    ChannelRecvError(RecvError),
    ChannelSendError(SendError<CancelMouseUpDelay>)
}

impl From<std::io::Error> for GtError {

    fn from(err: std::io::Error) -> Self {
        GtError::EventWriteError(err)
    }
}

impl From<VtpError> for GtError {
    fn from(err: VtpError) -> Self {
        match err {
            VtpError::EventWriteError(ioe) => GtError::EventWriteError(ioe), 
            VtpError::ChannelRecvError(RecvError) => GtError::ChannelRecvError(RecvError)
        }
    }
}

impl From<SendError<CancelMouseUpDelay>> for GtError {

    fn from(err: SendError<CancelMouseUpDelay>) -> Self {
        GtError::ChannelSendError(err)
    } 
}


pub struct GestureTranslator<'ex> {
    pub vtp: VirtualTrackpad,
    configs: Configuration,
    rt_exec: smol::Executor<'ex>,
    tx: Sender<CancelMouseUpDelay>
}

impl<'a> GestureTranslator<'a> {
    
    pub fn new(
        vtp: VirtualTrackpad, 
        configs: Configuration, 
        rt_exec: smol::Executor<'a>,
        tx: Sender<CancelMouseUpDelay>
    ) -> GestureTranslator<'a> {

        GestureTranslator {
            vtp,
            rt_exec,
            configs,
            tx
        }
    }

    pub async fn translate_gesture(&mut self, event: Event) -> Result<(), GtError> {

        debug!("Event received: {:?}", event);

        // early cancel of drag_end_delay is cfg'able.
        // if we don't check for the events that cancel it, no cancel 
        // signal will be sent, and the timer will run out naturally.
        if self.configs.drag_end_delay_cancellable {
            // check if the current event is worth sending a cancel signal in a channel 
            // to VirtualTrackpad::mouse_up_delay(), which is possibly running in another thread
            self.check_for_delay_cancelling_event(&event).await?;
        }

        match event {
            Event::Gesture(gest_ev) => {
                debug!("Handling gesture event");
                debug!("Number of fingers in gesture: {}", gest_ev.finger_count());
                
                if gest_ev.finger_count() != 3 { 
                    debug!("Gesture is not three-fingered, ignoring");
                    return self.mouse_up();
                }
                
                match gest_ev {
                    GestureEvent::Swipe(swipe_ev) => self.handle_swipe(swipe_ev).await,
                    GestureEvent::Hold(gest_hold_ev) => self.handle_hold(gest_hold_ev).await,
                    _ => self.mouse_up() // just in case, so the drag isn't locked
                }
            },
            _ => self.mouse_up()
        }
    }


    /// Check for the kinds of events that will cancel a drag end delay.
    /// There has got to be a better name for this function.
    async fn check_for_delay_cancelling_event(&self, ev: &Event) -> Result<(), SendError<CancelMouseUpDelay>>{
        
        // check whether cancellation-worthy events are detected,
        // send the signal into the channel if so
        debug!("checking for cancel-worthy event...");
        match ev {
            Event::Pointer(ptr_ev) => {
                match ptr_ev {
                    PointerEvent::Button(_) | PointerEvent::ScrollFinger(_) => {
                        debug!("cancelling, due to pointer click or scroll");
                        self.send_cancel_signal().await
                    },
                    _ => {
                        debug!("didn't find cancel-worthy event");
                        Ok(())
                    }
                }
            },
            Event::Gesture(gstr_ev) => {

                if gstr_ev.finger_count() < 3 {
                    debug!("cancelling, due to non-three-finger gesture");
                    return self.send_cancel_signal().await;
                }

                debug!("didn't find cancel-worthy event");
                Ok(())
            }
            _ => {
                debug!("didn't find cancel-worthy event");
                Ok(())
            }
        }
    }


    async fn handle_hold(&mut self, gest_hold_ev: GestureHoldEvent) -> Result<(), GtError> {

        debug!("handling hold");

        match gest_hold_ev {
            GestureHoldEvent::Begin(_) => self.mouse_down(),
            GestureHoldEvent::End(_) => {

                // don't waste time forking if there's no delay
                if self.configs.drag_end_delay.is_zero() { return self.mouse_up(); }

                self.fork_mouse_up_delay().await
            },
            _ => self.mouse_up()
        }
    }


    async fn handle_swipe(&mut self, swipe_ev: GestureSwipeEvent) -> Result<(), GtError> {
        
        debug!("handling swipe");
        
        match swipe_ev {
            GestureSwipeEvent::Update(swipe_update) => self.handle_swipe_update(swipe_update),
            GestureSwipeEvent::Begin(_) => self.mouse_down(),
            GestureSwipeEvent::End(_)   => {
                
                // don't waste time forking if there's no delay
                if self.configs.drag_end_delay.is_zero() { return self.mouse_up(); }

                self.fork_mouse_up_delay().await
            },
            _ => self.mouse_up()
        }
    }


    fn handle_swipe_update(&self, swipe_update: GestureSwipeUpdateEvent) -> Result<(), GtError> {
        
        debug!("handling GestureSwipeUpdate"); 
        
        let (dx, dy) = (
            swipe_update.dx_unaccelerated(), 
            swipe_update.dy_unaccelerated()
        );

        // Ignore tiny motions. This helps reduce drift.
        if dx.abs() < self.configs.min_motion 
        && dy.abs() < self.configs.min_motion {
            return Ok(());
        }

        self.vtp.mouse_move_relative(
            dx * self.configs.acceleration, 
            dy * self.configs.acceleration
        )?;

        Ok(())
    }


    /// Run the `mouse_up_delay` function asynchronously.
    async fn fork_mouse_up_delay(&mut self) -> Result<(), GtError> {

        // these are cheap clones; `VirtualTrackpad`'s clone is simply
        // duplicating a file descriptor (thin wrapper over 1 libc function) 
        // and getting another reference to the same channel receiver, 
        // and I'm only cloning the `std::time::Duration` part of the config.
        //
        // Both these clones together only add less than a microsecond to each 
        // iteration, which nothing when the usual response set in the main loop is 5ms 
        // -- more than 5000x longer. Run `cargo bench` for a demo. 
        let mut vtp_clone = self.vtp.clone();
        let delay = self.configs.drag_end_delay.clone();
        
        self.rt_exec.run(async move {
            vtp_clone
                .mouse_up_delay(delay)
                .await
                .map_err(GtError::from)
        }).await?;

        // since the boolean is copied with `vtp_clone`, the state needs 
        // to be updated here, as it's not in sync with the boolean that
        // gets updated in the async closure called in `Executor::run()` 
        // above
        self.vtp.mouse_is_down = false;

        Ok(())
    }


    /* Wrapper functions */
    
    fn mouse_down(&mut self) -> Result<(), GtError> {
        debug!("mouse_down called");
        self.vtp.mouse_down().map_err(GtError::from)
    }


    fn mouse_up(&mut self) -> Result<(), GtError> {
        debug!("vanilla mouse_up called");

        self.vtp.mouse_up().map_err(GtError::from)
    }

    async fn send_cancel_signal(&self) -> Result<(), SendError<CancelMouseUpDelay>> {
        self.rt_exec.run(
            self.tx.send(CancelMouseUpDelay)
        ).await
    }
}