use std::time::Duration;

//use smol::{channel::{RecvError, SendError, Sender}};
use tokio::sync::mpsc::{error::SendError, Sender};

use tracing::trace;

use super::virtual_trackpad::VirtualTrackpad;
use super::super::init::config::Configuration;

/// A signal to send into channel to control the behavior
/// of the listener on the separate thread that controls
/// when the mouse hold is released. Here's what each signal
/// means in more detail:
///
/// `CancelTimer`: Cancel the timer, and release drag inside fork
///
/// `CancelMouseUp`: Cancel timer, and don't do anything else in the fork (await next signal)
///
/// `RestartTimer`: Restart timer by restarting the loop in the fork that starts with a timer
///
/// `TerminateThread`: Terminate function running in fork
#[derive(Debug)]
pub enum ControlSignal {
    CancelTimer,      // currently not sent in practice, but could be without issue
    CancelMouseUp,
    RestartTimer,     // these two end up being treated the same in practice,
    TerminateThread
}

// (G)esture (T)ranslation Error
#[derive(Debug)]
pub enum GtError {
    EventWriteError(std::io::Error),
    JoinError(tokio::task::JoinError),
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

impl From<SendError<ControlSignal>> for GtError {

    fn from(err: SendError<ControlSignal>) -> Self {
        GtError::ChannelSendError(err)
    }
}


/// Drives the drag emulation (the virtual mouse). Gesture *detection* now
/// lives in `mt_proxy`, which owns the real trackpad and decides when a
/// 3-finger sequence begins/moves/ends; this struct just carries out the
/// resulting mouse_down/move/up primitives, same as before.
pub struct GestureTranslator {
    pub vtp: VirtualTrackpad,
    pub cfg: Configuration,
    tx: Sender<ControlSignal>,
}

impl GestureTranslator {

    pub fn new(
        vtp: VirtualTrackpad,
        cfg: Configuration,
        tx: Sender<ControlSignal>
    ) -> GestureTranslator {

        GestureTranslator {
            vtp,
            cfg,
            tx
        }
    }


    pub(crate) async fn update_cursor_position(&mut self, dx: f64, dy: f64) -> Result<(), GtError> {

        trace!("Moving cursor...");
        // if the cursor is moving during a drag, we don't want
        // the drag hold being randomly released
        self.send_signal(ControlSignal::CancelMouseUp).await?;

        self.vtp.mouse_move_relative(
            dx * self.cfg.acceleration,
            dy * self.cfg.acceleration
        )?;

        Ok(())
    }


    /// Sets mouse to down immediately, and cancels background
    /// `mouse_up_delay` timer.
    pub(crate) async fn mouse_down(&mut self) -> Result<(), GtError> {

        self.send_signal(ControlSignal::CancelMouseUp).await?;

        self.vtp
            .mouse_down()
            .map_err(GtError::from)
    }


    /// Handles the logic of calling the right function for
    /// releasing the mouse down state, to simplify functions
    /// further up the call stack.
    pub(crate) async fn handle_mouse_up(&mut self) -> Result<(), GtError> {

        // don't bother with forking and all that if there is
        // no delay to begin with
        if self.cfg.drag_end_delay == Duration::ZERO {

            return self.mouse_up_now().await;
        }

        // default case
        self.send_signal(ControlSignal::RestartTimer).await
    }


    /// Cancels the drag, cutting off any currently running delay.
    /// The left click is released here, not in the fork when the
    /// timer is running to cut down on latency.
    pub(crate) async fn mouse_up_now(&mut self) -> Result<(), GtError> {
        trace!("Cancelling timer, ending drag immediately");
        self.send_signal(ControlSignal::CancelMouseUp).await?;
        Ok(self.vtp.mouse_up()?)
    }


    /// Wrapper to send signal into channel.
    pub async fn send_signal(&mut self, sig: ControlSignal) -> Result<(), GtError> {

        // The channel can only hold a few messages (I chose to give it a
        // low bound), and this send will block until there is space in the
        // channel.
        trace!("Sending signal: {:?}", sig);
        self.tx.send(sig).await?;
        trace!("Signal sent!");
        Ok(())
    }
}
