use std::time::Duration;
use std::future::Future;
use smol::{channel::{RecvError, SendError, Sender}};
use input::{
    event::{
        gesture::{
            GestureEvent, 
            GestureEventCoordinates, 
            GestureEventTrait, 
            GestureHoldEvent, 
            GestureSwipeEvent
        }
    }, Event
};


use tracing::{debug, error, trace};

use super::virtual_trackpad::{VirtualTrackpad, VtpError};
use super::super::init::config::Configuration;

/// A signal to send into channel to control the behavior
/// of the listener on the separate thread that controls
/// when the mouse hold is released.
#[derive(Debug)]
pub enum ControlSignal {
    CancelTimer,
    CancelMouseUp,
    RestartTimer,     // these two end up being treated the same in practice,
    TerminateThread
}

// (G)esture (T)ranslation Error
#[derive(Debug)]
pub enum GtError {
    EventWriteError(std::io::Error),
    ChildThreadPanicked(std::io::Error),
    JoinError(tokio::task::JoinError),
    ChannelRecvError(RecvError),
    ChannelSendError(SendError<ControlSignal>)
}

impl From<std::io::Error> for GtError {

    fn from(err: std::io::Error) -> Self {
        GtError::EventWriteError(err)
    }
}

impl From<tokio::task::JoinError> for GtError {

    fn from(err: tokio::task::JoinError) -> Self {
        GtError::JoinError(err)
    }
}

impl From<RecvError> for GtError {
    fn from(err: RecvError) -> Self {
        GtError::ChannelRecvError(err)
    }
}

impl From<SendError<ControlSignal>> for GtError {

    fn from(err: SendError<ControlSignal>) -> Self {
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


/// Classic version of the program, where drag-end delays cannot be cancelled.
/// Useful for continuing drags with single fingers.
pub struct TranslatorClassic {
    pub vtp: VirtualTrackpad,
    pub cfg: Configuration,
}

/// Gesture Translator, **c**ancellable **d**elay. Allows for cancellable drag-end delays.
pub struct TranslatorCd {
    pub vtp: VirtualTrackpad,
    pub cfg: Configuration,
    pub tx: Sender<ControlSignal>,
}

pub trait GestureTranslator {
    fn translate_gesture(&mut self, event: Event) -> impl Future;
    fn send_signal(&mut self, sig: ControlSignal) -> impl Future;
    fn delay_cancellable(&self) -> bool;
    fn drag_end_delay(&self) -> Duration;
    fn response_time(&self) -> Duration;
    fn clone_vtp(&self) -> VirtualTrackpad;
}


#[allow(refining_impl_trait)]
impl GestureTranslator for TranslatorCd {

    async fn translate_gesture(&mut self, event: Event) -> Result<(), GtError> {
    
        debug!("Event received: {:?}", event);

        match event {
            Event::Gesture(gest_ev) => {

                // we don't care about gestures with other finger-counts
                if gest_ev.finger_count() != 3 {
                    debug!("Gesture not three-fingered, releasing drag");
                    return self.mouse_up_now().await;
                }
            
                match gest_ev {

                    GestureEvent::Hold(gest_hold_ev) => self.handle_hold(gest_hold_ev).await,
                    GestureEvent::Swipe(swipe_ev) => self.handle_swipe(swipe_ev).await,
                    _ => self.mouse_up_now().await // just in case, so the drag isn't locked
                }
            },
            _ => self.mouse_up_now().await
        }
    }


    /// Cancels the drag, cutting off any currently running delay.
    /// The left click is released via the fork wh
    async fn send_signal(&mut self, sig: ControlSignal) -> Result<(), GtError> {
        
        // The channel can only hold one message, and if one is 
        // already there, let it be consumed first. This should
        // all be synchronized enough to not have this happen, 
        // so if it does, raise an error.
        if self.tx.is_full() { 
            error!("Could not send {:?}: Channel has a signal in it already.", sig);
            return Err(GtError::ChannelSendError(SendError(sig))) 
        }
        
        trace!("Sending signal: {:?}", sig);
        self.tx.send(sig).await?;
        Ok(())
    }

    fn drag_end_delay(&self) -> Duration {
        self.cfg.drag_end_delay
    }
    
    fn delay_cancellable(&self) -> bool {
        self.cfg.drag_end_delay_cancellable
    }

    fn response_time(&self) -> Duration {
        self.cfg.response_time
    }

    /// Clones vitrual trackpad.
    fn clone_vtp(&self) -> VirtualTrackpad {
        self.vtp.clone()
    }
}


// Helper functions for public trait functions,
// with multi-threaded-specific implementations
impl TranslatorCd {

    async fn update_cursor_position(&mut self, dx: f64, dy: f64) -> Result<(), GtError> {

        trace!("Moving cursor...");
        // if the cursor is moving during a drag, we don't want
        // the drag hold being randomly released
        if self.cfg.drag_end_delay_cancellable {
            self.send_signal(ControlSignal::CancelMouseUp).await?;
        }

        // Ignore tiny motions. This helps improve stability of gesture.
        if dx.abs() < self.cfg.min_motion 
        && dy.abs() < self.cfg.min_motion {
            return Ok(());
        }

        self.vtp.mouse_move_relative(
            dx * self.cfg.acceleration, 
            dy * self.cfg.acceleration
        )?;

        Ok(())
    }


    async fn handle_hold(&mut self, hold_ev: GestureHoldEvent) -> Result<(), GtError> {
        match hold_ev {
            GestureHoldEvent::Begin(_) => self.mouse_down().await,
            GestureHoldEvent::End(_)   => self.handle_mouse_up().await,
            _ => self.mouse_up_now().await
        }
    }


    async fn handle_swipe(&mut self, swipe_ev: GestureSwipeEvent) -> Result<(), GtError> {
                    
        match swipe_ev {
            GestureSwipeEvent::Update(swipe_update) => {            
                self.update_cursor_position(
                    swipe_update.dx_unaccelerated(), 
                    swipe_update.dy_unaccelerated()
                ).await
            }
            GestureSwipeEvent::Begin(_) => self.mouse_down().await,
            GestureSwipeEvent::End(_)   => self.handle_mouse_up().await,
            _ => self.mouse_up_now().await
        }
    }


    /// Sets mouse to down immediately, and cancels background
    /// `mouse_up_delay` timer.
    async fn mouse_down(&mut self) -> Result<(), GtError> {
        
        self.send_signal(ControlSignal::CancelMouseUp).await?;
        Ok(self.vtp.mouse_down()?)
    }


    /// Handles the logic of calling the right function for 
    /// releasing the mouse down state, to simplify functions
    /// further up the call stack.
    async fn handle_mouse_up(&mut self) -> Result<(), GtError> {

        // don't bother with forking and all that if there is
        // no delay to begin with
        if self.cfg.drag_end_delay == Duration::ZERO {
            
            return self.mouse_up_now().await;
        }

        // default case
        self.send_signal(ControlSignal::RestartTimer).await
    }


    /// Cancels the drag, cutting off any currently running delay.
    /// The left click is released via the fork when it's running,
    /// finishing the task and resetting `spandle` to `None.`
    async fn mouse_up_now(&mut self) -> Result<(), GtError> {
        
        trace!("Cancelling timer, ending drag immediately");
        self.send_signal(ControlSignal::CancelTimer).await
    }
}


#[allow(refining_impl_trait)]
impl GestureTranslator for TranslatorClassic {

    async fn translate_gesture(&mut self, event: Event) -> Result<(), GtError> {
    
        debug!("Event received: {:?}", event);

        match event {
            Event::Gesture(gest_ev) => {

                // we don't care about gestures with other finger-counts
                if gest_ev.finger_count() != 3 {
                    debug!("Gesture not three-fingered, releasing drag");
                    return Ok(());
                }
            
                match gest_ev {

                    GestureEvent::Hold(gest_hold_ev) => self.handle_hold(gest_hold_ev).await,
                    GestureEvent::Swipe(swipe_ev) => self.handle_swipe(swipe_ev).await,
                    _ => Ok(())
                }
            },
            _ => Ok(())
        }
    }

    /// Nothing to send the signal to, so simply is implemented for the trait.
    async fn send_signal(&mut self, _sig: ControlSignal) -> Result<(), GtError> {
        
        Ok(())
    }

    fn drag_end_delay(&self) -> Duration {
        self.cfg.drag_end_delay
    }
    
    fn delay_cancellable(&self) -> bool {
        self.cfg.drag_end_delay_cancellable
    }

    fn response_time(&self) -> Duration {
        self.cfg.response_time
    }

    /// Clones vitrual trackpad.
    fn clone_vtp(&self) -> VirtualTrackpad {
        self.vtp.clone()
    }
}


impl TranslatorClassic {
    fn update_cursor_position(&mut self, dx: f64, dy: f64) -> Result<(), GtError> {

        trace!("Moving cursor...");

        // Ignore tiny motions. This helps improve stability of gesture.
        if dx.abs() < self.cfg.min_motion 
        && dy.abs() < self.cfg.min_motion {
            return Ok(());
        }

        self.vtp.mouse_move_relative(
            dx * self.cfg.acceleration, 
            dy * self.cfg.acceleration
        )?;

        Ok(())
    }


    async fn handle_hold(&mut self, hold_ev: GestureHoldEvent) -> Result<(), GtError> {
        match hold_ev {
            GestureHoldEvent::Begin(_) => self.mouse_down(),
            GestureHoldEvent::End(_)   => self.handle_mouse_up(),
            _ => self.mouse_up_now()
        }
    }


    async fn handle_swipe(&mut self, swipe_ev: GestureSwipeEvent) -> Result<(), GtError> {
                    
        match swipe_ev {
            GestureSwipeEvent::Update(swipe_update) => {            
                self.update_cursor_position(
                    swipe_update.dx_unaccelerated(), 
                    swipe_update.dy_unaccelerated()
                )
            }
            GestureSwipeEvent::Begin(_) => self.mouse_down(),
            GestureSwipeEvent::End(_)   => self.handle_mouse_up(),
            _ => self.mouse_up_now()
        }
    }


    /// Sets mouse to down immediately, and cancels background
    /// `mouse_up_delay` timer.
    fn mouse_down(&mut self) -> Result<(), GtError> {
        
        Ok(self.vtp.mouse_down()?)
    }


    /// Handles the logic of calling the right function for 
    /// releasing the mouse down state, to simplify functions
    /// further up the call stack.
    fn handle_mouse_up(&mut self) -> Result<(), GtError> {

        // don't bother with forking and all that if there is
        // no delay to begin with
        if self.cfg.drag_end_delay == Duration::ZERO {
            
            return self.mouse_up_now();
        }

        // default case
        Ok(self.vtp.mouse_up_delay_blocking(self.cfg.drag_end_delay)?)
    }


    /// Wrapper for `VirtualTrackpad::mouse_up`.
    fn mouse_up_now(&mut self) -> Result<(), GtError> {
        
        Ok(self.vtp.mouse_up()?)
    }
}