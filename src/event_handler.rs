use input::{
    event::{
        Event,
        gesture::{
        GestureEvent, 
            GestureEventCoordinates,
            GestureEventTrait,
            GestureHoldEvent,
            GestureSwipeEvent
        }, 
        PointerEvent
    }
};
use super::virtual_trackpad::VirtualTrackpad;
use super::config::Configuration;


// Keeping this separate to centralize the config'd behavior to one place
fn update_mouse(vtrackpad: &mut VirtualTrackpad, configs: &Configuration, dx: f64, dy: f64) -> Result<(), std::io::Error> {
    
    // Ignore tiny motions. This helps improve stability of gesture.
    if dx.abs() < configs.min_motion && dy.abs() < configs.min_motion {
        return Ok(());
    }

    vtrackpad.mouse_move_relative(
        dx * configs.acceleration, 
        dy * configs.acceleration
    )?;

    Ok(())
}

pub fn translate_gesture(event: Event, vtrackpad: &mut VirtualTrackpad, configs: &Configuration) -> Result<(), std::io::Error> {
    
    match event {
        Event::Gesture(gest_ev) => {

            // we don't care about gestures with other finger-counts
            if gest_ev.finger_count() != 3 {return Ok(());}
           
            match gest_ev {

                GestureEvent::Hold(gest_hold_ev) => {
                    match gest_hold_ev {
                        GestureHoldEvent::Begin(_) => vtrackpad.mouse_down(),
                        GestureHoldEvent::End(_)   => vtrackpad.mouse_up_delay(configs.drag_end_delay),
                        _ => Ok(())
                    }
                },
                GestureEvent::Swipe(swipe_ev) => {
                    
                    match swipe_ev {
                        GestureSwipeEvent::Update(swipe_update) => {
                            
                            let (dx, dy) = (
                                swipe_update.dx_unaccelerated(), 
                                swipe_update.dy_unaccelerated()
                            );

                            update_mouse(vtrackpad, configs, dx, dy)?;

                            Ok(())
                        }
                        GestureSwipeEvent::Begin(_) => vtrackpad.mouse_down(),
                        GestureSwipeEvent::End(_)   => vtrackpad.mouse_up_delay(configs.drag_end_delay),
                        _ => Ok(())
                    }
                }
                _ => vtrackpad.mouse_up() // just in case, so the drag isn't locked
            }
        },
        Event::Pointer(point_ev) => {

            // only update mouse position from pointer events when virtual trackpad's
            // mouse is down. Otherwise pointer motion events are written to the real
            // trackpad twice, resulting in double-speed pointer motion outside of 
            // three-finger dragging.
            if !vtrackpad.mouse_is_down {return Ok(())}

            match point_ev {
                PointerEvent::Motion(motion_ev) => {
                    
                    let (dx, dy) = (
                        motion_ev.dx_unaccelerated(), 
                        motion_ev.dy_unaccelerated()
                    );

                    update_mouse(vtrackpad, configs, dx, dy)?;

                    Ok(())
                },
                _ => Ok(())
            }
        },
        _ => Ok(())
    }
}