use input::{
    Event,
    event::gesture::{
    GestureEvent, 
        GestureEventCoordinates,
        GestureEventTrait,
        GestureHoldEvent,
        GestureSwipeEvent
    }
};
use crate::virtual_trackpad::VirtualTrackpad;
use crate::init_fns::config::Configuration;


pub fn translate_gesture(event: Event, vtrackpad: &mut VirtualTrackpad, configs: &Configuration) {
    
    match event {
        Event::Gesture(gest_ev) => {

            // we don't care about gestures with other finger-counts
            if gest_ev.finger_count() != 3 {return;}
           
            match gest_ev {

                GestureEvent::Hold(gest_hold_ev) => {
                    match gest_hold_ev {
                        GestureHoldEvent::Begin(_) => vtrackpad.mouse_down(),
                        GestureHoldEvent::End(_)   => vtrackpad.mouse_up_delay(configs.drag_end_delay),
                        _ => {}
                    }
                },
                GestureEvent::Swipe(swipe_ev) => {
                    
                    match swipe_ev {
                        GestureSwipeEvent::Update(swipe_update) => {
                            
                            let (dx, dy) = (
                                swipe_update.dx_unaccelerated(), 
                                swipe_update.dy_unaccelerated()
                            );

                            // Ignore tiny motions. This helps reduce drift.
                            if dx.abs() < configs.min_motion && dy.abs() < configs.min_motion {return;}

                            vtrackpad.mouse_move_relative(
                                dx * configs.acceleration, 
                                dy * configs.acceleration
                            );
                        }
                        GestureSwipeEvent::Begin(_) => vtrackpad.mouse_down(),
                        GestureSwipeEvent::End(_)   => vtrackpad.mouse_up_delay(configs.drag_end_delay),
                        _ => {}
                    }
                }
                _ => vtrackpad.mouse_up() // just in case, so the drag isn't locked
            }
        }
        _ => {}
    }
    
}