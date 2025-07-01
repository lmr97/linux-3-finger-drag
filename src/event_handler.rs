use input::event::gesture::{
    GestureEvent, 
    GestureEventCoordinates,
    GestureHoldEvent,
    GestureSwipeEvent
};
use crate::virtual_trackpad::VirtualTrackpad;
use crate::init_fns::config::Configuration;


pub fn translate_gesture(gesture_event: GestureEvent, vtrackpad: &mut VirtualTrackpad, configs: &Configuration) {

    match gesture_event {
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
                    vtrackpad.mouse_move_relative(
                        swipe_update.dx() * configs.acceleration, 
                        swipe_update.dy() * configs.acceleration
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