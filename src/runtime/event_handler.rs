use std::any::Any;
use std::io::ErrorKind;
use std::mem;
use std::sync::{Arc, PoisonError, MutexGuard};
use smol::Task;
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

use super::virtual_trackpad::VirtualTrackpad;
use super::super::init::config::Configuration;

type MutexPoisonError<'e> = PoisonError<MutexGuard<'e, Arc<VirtualTrackpad>>>;

// (G)esture (T)ranslation Error
#[derive(Debug)]
#[allow(dead_code)]  // Rust complains that I don't read the inner errors
pub enum GtError {
    CannotWriteEvent(std::io::Error),
    ChildThreadPanicked(std::io::Error),
}

impl From<std::io::Error> for GtError {

    fn from(err: std::io::Error) -> Self {
        GtError::CannotWriteEvent(err)
    }
}

impl From<MutexPoisonError<'_>> for GtError {

    fn from(err: MutexPoisonError<'_>) -> Self {

        let err_msg = format!(
            "Mutex was poisoned (a locking thread panicked): {:?}", err
        );

        let recast_err = std::io::Error::new(
            ErrorKind::Other,
            err_msg
        );

        GtError::ChildThreadPanicked(recast_err)
    }
}


struct TaskHandle { task: Task<Result<(), GtError>> }

impl Default for TaskHandle {
    fn default() -> Self {
        TaskHandle {
            task: smol::spawn(async { 
                Ok(()) 
            })
        }
    }
}

impl TaskHandle {
    // executing a block_on runtime from the main thread causes a 
    // deadlock
    fn cancel(self) -> Option<Result<(), GtError>> {
        smol::block_on(async move {
            
            debug!("Got into cancelation runtime");
            if self.task.is_finished() { 
                debug!(
                    "Task was found to be already finished in \
                    cancellation runtime, exiting runtime..."
                );
                return Some(Ok(())); 
            }
            
            let opt_res = self.task.cancel().await;
            debug!("Option received: {:?}", opt_res);
            debug!("Cancellation complete!");
            
            opt_res
        })
    }
}

pub struct GestureTranslator<'ex> {
    vtp: Arc<VirtualTrackpad>,
    configs: Arc<Configuration>,
    rt_exec: smol::Executor<'ex>,
    mud_child: TaskHandle,  // mouse_up_delay child
    mouse_is_down: bool     // easier to track here than in VTP, due to Arcs/borrows
}

impl<'a> GestureTranslator<'a> {
    
    pub fn new(
        vtp: VirtualTrackpad, 
        cfg: Configuration, 
        rt_exec: smol::Executor<'a>
    ) -> GestureTranslator<'a> {
        
        GestureTranslator {
            vtp: Arc::new(vtp),
            rt_exec,
            configs: Arc::new(cfg),
            mud_child: TaskHandle::default(),
            mouse_is_down: false
        }
    }

    pub fn translate_gesture(&mut self, event: Event) -> Result<(), GtError> {

        debug!("Event received: {:?}", event);
        match event {
            Event::Gesture(gest_ev) => {
                debug!("Handling gesture event");
                debug!("Type of gesture event is GestureHoldEvent: {}", std::any::TypeId::of::<GestureHoldEvent>() == gest_ev.type_id());
                debug!("Number of fingers in gesture: {}", gest_ev.finger_count());
           
                match gest_ev {
                    GestureEvent::Swipe(swipe_ev) => self.handle_swipe(swipe_ev),
                    GestureEvent::Hold(gest_hold_ev) => {
                        debug!("FOUND HOLD EVENT!");
                        self.handle_hold(gest_hold_ev)
                    },
                    _ => {
                        debug!("not matched on Swipe or Hold");
                        self.mouse_up_now()
                    } // just in case, so the drag isn't locked
                }
            }
            //Event::Pointer(pointer_ev) => self.handle_pointer_ev(pointer_ev),
            _ => {
                debug!("not matched on Gesture or Pointer");
                self.mouse_up_now()
            }
        }
    }


    fn handle_hold(&mut self, gest_hold_ev: GestureHoldEvent) -> Result<(), GtError> {

        debug!("IN HOLD HANDLER");
        match gest_hold_ev {
            GestureHoldEvent::Begin(_) => self.mouse_down(),
            GestureHoldEvent::End(_) => {

                if self.configs.drag_end_delay.is_zero() { 
                    return self.mouse_up();
                }

                // these are cheap clones; `VirtualTrackpad`'s clone is simply
                // duplicating a file descriptor (thin wrapper over 1 libc function) 
                // and `Configuration`'s clone is on a small struct of small data
                // the largest is a `std::time::Duration`. Both these clones together
                // only add less than a microsecond to each iteration, and when the 
                // the usual response set in the main loop is 5ms -- >5000x longer --
                // the time taken cloning will make practically no difference.
                // (see `src/benches/cloning.rs` for a demo).
                let vtp_arc = self.vtp.clone();
                let delay = self.configs.drag_end_delay.clone();

                self.mud_child.task = self.rt_exec.spawn(async move {
                    vtp_arc
                        .mouse_up_delay(delay)
                        .map_err(GtError::from)
                });
                debug!(
                    "runtime is empty after fork (from handle_hold): {}", 
                    self.rt_exec.is_empty()
                );
                
                Ok(())
            },
            _ => self.mouse_up_now()
        }
    }


    fn handle_swipe(&mut self, swipe_ev: GestureSwipeEvent) -> Result<(), GtError> {
        debug!("handling swipe");
        if swipe_ev.finger_count() != 3 { 
            return self.mouse_up_now();
        }
        match swipe_ev {
            GestureSwipeEvent::Update(swipe_update) => self.handle_swipe_update(swipe_update),
            GestureSwipeEvent::Begin(_) => {debug!("handling GestureSwipeBegin"); self.mouse_down()},
            GestureSwipeEvent::End(_)   => {
                
                if self.configs.drag_end_delay.is_zero() { 
                    return self.mouse_up();
                }

                let vtp_arc = self.vtp.clone();
                let delay = self.configs.drag_end_delay.clone();

                self.mud_child.task = self.rt_exec.spawn(async move {
                    vtp_arc
                        .mouse_up_delay(delay)
                        .map_err(GtError::from)
                });

                debug!(
                    "runtime is empty after fork (from handle_swipe): {}", 
                    self.rt_exec.is_empty()
                );
                Ok(())
            },
            _ => self.mouse_up_now()
        }
    }


    fn handle_swipe_update(&self, swipe_update: GestureSwipeUpdateEvent) -> Result<(), GtError> {
        
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


    fn handle_pointer_ev(&mut self, p_ev: PointerEvent) -> Result<(), GtError> {
        match p_ev {
            PointerEvent::Motion(mot_ev) => {
                
                if !self.mouse_is_down { return self.mouse_up_now(); }
                
                let (dx, dy) = (
                    mot_ev.dx_unaccelerated(), 
                    mot_ev.dy_unaccelerated()
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
            },
            _ => self.mouse_up_now()
        }
    }

    /* Wrapper functions */
    
    fn mouse_down(&mut self) -> Result<(), GtError> {
        debug!("mouse_down called");
        self.mouse_is_down = true;
        self.vtp.mouse_down().map_err(GtError::from)
    }


    fn mouse_up(&mut self) -> Result<(), GtError> {
        debug!("vanilla mouse_up called");
        self.mouse_is_down = false;
        self.vtp.mouse_up().map_err(GtError::from)
    }

    #[track_caller]
    fn mouse_up_now(&mut self) -> Result<(), GtError> {
        debug!("Caller: {:?}", std::panic::Location::caller().line());
        debug!("Starting cancel of mouse_up_delay...");
        let mud_child = mem::take(&mut self.mud_child);
        debug!("Task is finished: {}", mud_child.task.is_finished());
        
        let cancel_res = match mud_child.cancel() {
            Some(res) => res,
            None => Ok(())
        };
        debug!("runtime is empty after cancel: {}", self.rt_exec.is_empty());
        self.mouse_up()?;

        cancel_res
    }
}