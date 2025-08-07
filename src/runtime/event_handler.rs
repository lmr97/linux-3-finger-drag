use std::thread::JoinHandle;

use smol::{Task, channel::{RecvError, SendError, Sender}};
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


use tracing::{debug, trace};

use super::virtual_trackpad::{VirtualTrackpad, VtpError};
use super::super::init::config::Configuration;

// the "signal" to send into channel to cancel 
#[derive(Debug)]
pub struct CancelSignal {
    #[allow(dead_code)]    // useful for debugging
    id: u64
}

// (G)esture (T)ranslation Error
#[derive(Debug)]
pub enum GtError {
    EventWriteError(std::io::Error),
    ChildThreadPanicked(std::io::Error),
    ChannelRecvError(RecvError),
    ChannelSendError(SendError<CancelSignal>)
}

impl From<std::io::Error> for GtError {

    fn from(err: std::io::Error) -> Self {
        GtError::EventWriteError(err)
    }
}

impl From<RecvError> for GtError {
    fn from(err: RecvError) -> Self {
        GtError::ChannelRecvError(err)
    }
}

impl From<SendError<CancelSignal>> for GtError {

    fn from(err: SendError<CancelSignal>) -> Self {
        GtError::ChannelSendError(err)
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


pub struct GestureTranslator<'ex> {
    pub vtp: VirtualTrackpad,
    configs: Configuration,
    rt: smol::Executor<'ex>,
    mud_child: Task<Result<(), VtpError>>,  // mouse_up_delay child handle
    spandle: Option<JoinHandle<Result<(), GtError>>>,
    tx: Sender<CancelSignal>,
    cxl_id: u64
}

impl<'e> GestureTranslator<'e> {
    
    pub fn new(
        vtp: VirtualTrackpad, 
        configs: Configuration, 
        rt: smol::Executor<'e>,
        tx: Sender<CancelSignal>
    ) -> GestureTranslator<'e> {

        let empty_task = rt.spawn(async { Ok(()) });
        GestureTranslator {
            vtp,
            configs,
            rt,
            mud_child: empty_task,
            tx,
            spandle: None,
            cxl_id: 0
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
                trace!("Handling gesture event");
                debug!("Number of fingers in gesture: {}", gest_ev.finger_count());
                
                if gest_ev.finger_count() != 3 { 
                    trace!("Gesture is not three-fingered, ignoring");
                    return self.mouse_up().await;
                }
                
                match gest_ev {
                    GestureEvent::Swipe(swipe_ev) => self.handle_swipe(swipe_ev).await,
                    GestureEvent::Hold(gest_hold_ev) => self.handle_hold(gest_hold_ev).await,
                    _ => self.mouse_up().await // just in case, so the drag isn't locked
                }
            },
            _ => self.mouse_up().await
        }
    }


    /// Check for the kinds of events that will cancel a drag end delay.
    /// There has got to be a better name for this function.
    async fn check_for_delay_cancelling_event(&mut self, ev: &Event) -> Result<(), SendError<CancelSignal>>{
        
        if !self.vtp.mouse_is_down { return Ok(()) }

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
                        trace!("didn't find cancel-worthy event");
                        Ok(())
                    }
                }
            },
            Event::Gesture(gstr_ev) => {

                if gstr_ev.finger_count() < 3 {
                    debug!("cancelling, due to non-three-finger gesture");
                    return self.send_cancel_signal().await;
                }

                trace!("didn't find cancel-worthy event");
                Ok(())
            }
            _ => {
                trace!("didn't find cancel-worthy event");
                Ok(())
            }
        }
    }


    async fn handle_hold(&mut self, gest_hold_ev: GestureHoldEvent) -> Result<(), GtError> {

        trace!("handling hold");

        match gest_hold_ev {
            GestureHoldEvent::Begin(_) => self.mouse_down(),
            GestureHoldEvent::End(_) => {

                // don't waste time forking if there's no delay
                if self.configs.drag_end_delay.is_zero() { 
                    return self.vtp.mouse_up().map_err(GtError::from); 
                }

                self.refresh_fork2()
            },
            _ => self.mouse_up().await
        }
    }


    async fn handle_swipe(&mut self, swipe_ev: GestureSwipeEvent) -> Result<(), GtError> {
        
        trace!("handling swipe");
        
        match swipe_ev {
            GestureSwipeEvent::Update(swipe_update) => self.handle_swipe_update(swipe_update),
            GestureSwipeEvent::Begin(_) => self.mouse_down(),
            GestureSwipeEvent::End(_)   => {
                
                // don't waste time forking if there's no delay
                if self.configs.drag_end_delay.is_zero() { 
                    return self.vtp.mouse_up().map_err(GtError::from); 
                }

                self.refresh_fork2()
            },
            _ => self.mouse_up().await
        }
    }


    fn handle_swipe_update(&self, swipe_update: GestureSwipeUpdateEvent) -> Result<(), GtError> {
        
        trace!("handling GestureSwipeUpdate"); 
        
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


    async fn send_cancel_signal(&mut self) -> Result<(), SendError<CancelSignal>> {
        // self.vtp.clear_buffer().unwrap();
        if !self.tx.is_empty() { return Ok(()) }

        self.cxl_id += 1;
        let sig = CancelSignal {id: self.cxl_id};
        debug!("sending cxl sig: {:?}", sig);
        
        self.tx.send(sig).await
    }

    fn empty_task(&self) -> Task<Result<(), VtpError>> {
        self.rt.spawn(async {
            Ok(())
        })
    }


    /// If this function raises an error, it is from the *previous* task
    /// spawned. I want the task to run as long as possible in the background,
    /// so any errors will only be found here, upon refreshing the fork.
    /// 
    /// Tasks are automatically cancelled when they are dropped, so cleanup 
    /// is not needed when the whole program exits.
    async fn refresh_fork(&mut self) -> Result<(), GtError> {
        trace!("refereshing fork...");
        let new_task = self.fork_mouse_up_delay();

        let prev_task = std::mem::replace(
            &mut self.mud_child, 
            new_task
        );

        self.rt.run(prev_task)
            .await
            .map_err(GtError::from)
    }

    fn refresh_fork2(&mut self) -> Result<(), GtError> {
        
        let vtp_clone = self.vtp.clone();
        let delay = self.configs.drag_end_delay.clone();

        let new_handle = std::thread::spawn(move || {
            smol::block_on(async {
                vtp_clone.mouse_up_delay(delay)
                    .await
                    .map_err(GtError::from)
            })
        });

        self.spandle = Some(new_handle);
        Ok(())
    }

    async fn clear_task2(&mut self) -> Result<(), GtError> {
        
        let prev_task = std::mem::replace(
            &mut self.spandle, 
            None
        );

        match prev_task {
            Some(t) => t.join().expect("Could not join thread."),
            None => Ok(())
        }
    }
    

    /// Runs the member task and replaces it with a dummy task
    /// that simply returns `Ok(())`.
    async fn clear_task(&mut self) -> Result<(), GtError> {
        let empty = self.empty_task();
        
        let prev_task = std::mem::replace(
            &mut self.mud_child, 
            empty
        );

        self.rt.run(prev_task)
            .await
            .map_err(GtError::from)
    }

    /// Run the `mouse_up_delay` function asynchronously. When this function
    /// raises an error, it is from the *previous* forked task.
    fn fork_mouse_up_delay(&mut self) -> Task<Result<(), VtpError>> {

        // These are very cheap clones (< 1us total). For 
        // specific values, you can run the benchmark test
        // with `cargo bench`. 
        let vtp_clone = self.vtp.clone();
        let delay = self.configs.drag_end_delay.clone();

        self.rt.spawn(
            async move {
                vtp_clone.mouse_up_delay(delay).await
            }
        )
    }


    /* Wrapper functions */
    
    fn mouse_down(&mut self) -> Result<(), GtError> {
        trace!("mouse_down wrapper called");
        self.vtp.mouse_down().map_err(GtError::from)
    }


    async fn mouse_up(&mut self) -> Result<(), GtError> {
        trace!("mouse_up wrapper called");
        //self.vtp.mouse_up().await.map_err(GtError::from)
        self.clear_task2().await
    }
}