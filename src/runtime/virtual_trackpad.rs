//! The virtual mouse that carries out the drag: a minimal uinput device
//! with one button and relative motion. All *timing* concerns (debounce
//! windows, drag-lock) live in the gesture machine -- this device just
//! writes what it's told, synchronously, in order.
//!
//! (Historical note: this used to host a cancellable-timer thread and a
//! control-signal channel to implement dragEndDelay. That machinery is
//! gone -- the delay is now a deadline inside the gesture state machine,
//! where it can be unit-tested and where "release the button *before*
//! relaying someone else's touch" is enforced by construction.)

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::{thread, time};

use input_linux::{
    EventKind, EventTime, InputEvent, InputId, Key, KeyEvent, KeyState, RelativeAxis,
    RelativeEvent, SynchronizeEvent, SynchronizeKind, UInputHandle,
};
use libc::O_NONBLOCK;
use tracing::{debug, error};

pub struct VirtualTrackpad {
    handle: UInputHandle<File>,
    pub mouse_is_down: bool,
    /// Which button a drag presses (see the `dragButton` config option).
    drag_button: Key,
    /// The button actually held right now, so a release always matches
    /// the press even if `dragButton` is hot-reloaded mid-drag.
    pressed: Option<Key>,
}

pub fn start_handler() -> Result<VirtualTrackpad, std::io::Error> {
    let uinput_file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_NONBLOCK)
        .open("/dev/uinput")
        .inspect_err(|_| {
            error!(
                "You are not yet allowed to write to /dev/uinput.\n\
                Some things to try:\n\
                - Update the udev rules for uinput (see installation guide in README.md, step 3.1)\n\
                - Log out and log in again\n\
                - Restart your computer\n\
                - FOR ARCH: make sure the uinput kernel module is loaded on boot\n",
            );
        })?;

    let uhandle = UInputHandle::new(uinput_file);

    uhandle.set_evbit(EventKind::Key)?;
    uhandle.set_keybit(Key::ButtonLeft)?;
    // Also advertise a right button, so a drag can press it instead of
    // the left one. GNOME/KDE's left-handed mouse setting swaps the
    // buttons of every mouse-type device -- including this one -- so
    // with that setting on, BTN_RIGHT is what arrives at apps as a left
    // press. (Advertising both also makes this look like an ordinary
    // mouse rather than an unusual one-button device.)
    uhandle.set_keybit(Key::ButtonRight)?;

    uhandle.set_evbit(EventKind::Relative)?;
    uhandle.set_relbit(RelativeAxis::X)?;
    uhandle.set_relbit(RelativeAxis::Y)?;

    let input_id = InputId {
        bustype: input_linux::sys::BUS_USB,
        vendor: 0x1234,
        product: 0x5678, // iykyk
        version: 0,
    };
    let device_name = b"Virtual trackpad (created by linux-3-finger-drag)";
    uhandle.create(&input_id, device_name, 0, &[])?;
    debug!("Virtual trackpad successfully created.");

    // may be needed to let the system catch up
    thread::sleep(time::Duration::from_millis(500));

    Ok(VirtualTrackpad {
        handle: uhandle,
        mouse_is_down: false,
        drag_button: Key::ButtonLeft,
        pressed: None,
    })
}

impl VirtualTrackpad {
    const ZERO: EventTime = EventTime::new(0, 0);

    fn syn() -> input_linux::sys::input_event {
        InputEvent::from(SynchronizeEvent::new(
            VirtualTrackpad::ZERO,
            SynchronizeKind::Report,
            0,
        ))
        .into_raw()
    }

    /// Choose which button drags press: `true` for BTN_RIGHT (for
    /// left-handed mouse setups), `false` for BTN_LEFT. Takes effect at
    /// the next press; a button already held is still released as-is.
    pub fn set_drag_button_right(&mut self, right: bool) {
        self.drag_button = if right {
            Key::ButtonRight
        } else {
            Key::ButtonLeft
        };
    }

    pub fn mouse_down(&mut self) -> Result<(), std::io::Error> {
        let key = self.pressed.unwrap_or(self.drag_button);
        let events = [
            InputEvent::from(KeyEvent::new(
                VirtualTrackpad::ZERO,
                key,
                KeyState::pressed(true),
            ))
            .into_raw(),
            Self::syn(),
        ];
        self.handle.write(&events)?;
        self.mouse_is_down = true;
        self.pressed = Some(key);
        Ok(())
    }

    pub fn mouse_up(&mut self) -> Result<(), std::io::Error> {
        let key = self.pressed.take().unwrap_or(self.drag_button);
        let events = [
            InputEvent::from(KeyEvent::new(
                VirtualTrackpad::ZERO,
                key,
                KeyState::pressed(false),
            ))
            .into_raw(),
            Self::syn(),
        ];
        self.handle.write(&events)?;
        self.mouse_is_down = false;
        debug!("virtual mouse button released");
        Ok(())
    }

    /// Whole-pixel relative motion. Sub-pixel remainders are carried by
    /// the gesture machine, so nothing is lost to truncation here.
    pub fn mouse_move_relative(&mut self, dx: i32, dy: i32) -> Result<(), std::io::Error> {
        let events = [
            InputEvent::from(RelativeEvent::new(
                VirtualTrackpad::ZERO,
                RelativeAxis::X,
                dx,
            ))
            .into_raw(),
            InputEvent::from(RelativeEvent::new(
                VirtualTrackpad::ZERO,
                RelativeAxis::Y,
                dy,
            ))
            .into_raw(),
            Self::syn(),
        ];
        self.handle.write(&events)?;
        Ok(())
    }

    pub fn destruct(self) -> Result<(), std::io::Error> {
        self.handle.dev_destroy()
    }
}
