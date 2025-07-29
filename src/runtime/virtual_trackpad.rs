// this file is basically copied and rearranged from arcnmx's GitHub example
// in the input-linux-rs repo (a translation of an example
// on the Linux kernel's uinput module, actually). 
// The Rust example can be found here: 
// https://github.com/arcnmx/input-linux-rs/blob/main/examples/mouse-movements.rs

use std::fs::{File, OpenOptions};
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;
use std::{thread, time};

use input_linux::{
    EventKind, EventTime, 
    InputEvent, InputId, 
    Key, KeyEvent, KeyState, 
    RelativeAxis, RelativeEvent, 
    SynchronizeEvent, SynchronizeKind, 
    UInputHandle
};
use nix::libc::O_NONBLOCK;
use log::{debug, error};

/// This struct is stateless: no position or mouse state is available.
/// This is due to issues that arise from mutability in a mulit-thread
/// context. If state is required, a wrapper struct will need to be
/// created, or tracked externally somehow. 
pub struct VirtualTrackpad {
    handle: UInputHandle<File>,
}

pub fn start_handler() -> Result<VirtualTrackpad, std::io::Error> {
    let uinput_file_res = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_NONBLOCK)
        .open("/dev/uinput");

    let uinput_file = match uinput_file_res {
        Ok(file) => file,
        Err(e) => {
            error!(
                "You are not yet allowed to write to /dev/uinput.\n\
                Some things to try:\n\
                - Update the udev rules for uinput (see installation guide in README.md, step 3.1)\n\
                - Log out and log in again\n\
                - Restart your computer\n\
                - FOR ARCH: make sure the uinput kernel module is loaded on boot\n",
            );
            return Err(e);
        }
    };

    let uhandle = UInputHandle::new(uinput_file);

    // I'm using unwraps here because this function is only called 
    // during the program's setup phase. I've also never had these 
    // functions below crash the program; if this `start_handler()`
    // ever crashes (from my experience), it's always an issue with
    // trying to read `/dev/uinput`. It's typically smooth sailing
    // in this function after that succeeds. 
    uhandle.set_evbit(EventKind::Key).unwrap();
    uhandle.set_keybit(input_linux::Key::ButtonLeft).unwrap();

    uhandle.set_evbit(EventKind::Relative).unwrap();
    uhandle.set_relbit(RelativeAxis::X).unwrap();
    uhandle.set_relbit(RelativeAxis::Y).unwrap();

    let input_id = InputId {
        bustype: input_linux::sys::BUS_USB,
        vendor: 0x1234,
        product: 0x5678,  // iykyk
        version: 0,
    };
    let device_name = b"Virtual trackpad (created by linux-3-finger-drag)";
    uhandle.create(&input_id, device_name, 0, &[]).unwrap();
    debug!("Virtual trackpad successfully created.");

    // may be needed to let the system catch up
    thread::sleep(time::Duration::from_millis(500));

    Ok(VirtualTrackpad { handle: uhandle })

}


impl Clone for VirtualTrackpad {
    /// This clone() can theoretically panic since there is an expect() in 
    /// its definition. This is because `try_cloned_to_owned`, from `std::io`,
    /// utilizes libc's `fnctl`, which will only fail if there is the maximum
    /// number of file descriptors to be opened is reached (or if the args
    /// to it are invalid, but the Rust method takes no arguments except for
    /// a known-valid FD, so the rest are controlled by the std library).
    /// 
    /// This makes it as safe as any other function to call, since it only fails
    /// when there is a resource limitation issue (which would be a rare and 
    /// system-wide problem).
    fn clone(&self) -> Self {
        let fd = self.handle
            .as_fd()
            .try_clone_to_owned()
            .expect(
                "uinput file descriptor could not be duplicated, \
                likely do to hitting the maximum open file descriptors \
                for this OS."
        );

        VirtualTrackpad {
            handle: UInputHandle::new(File::from(fd))
        }
    }
}

impl VirtualTrackpad
{
    const ZERO: EventTime = EventTime::new(0, 0);

    pub fn mouse_down(&self) -> Result<(), std::io::Error> {
        let events = [
            InputEvent::from(
                KeyEvent::new(
                    VirtualTrackpad::ZERO, 
                    Key::ButtonLeft, 
                    KeyState::pressed(true))
                ).into_raw(),
            InputEvent::from(
                SynchronizeEvent::new(
                    VirtualTrackpad::ZERO, 
                    SynchronizeKind::Report, 
                    0)
                ).into_raw(),
        ];
        self.handle.write(&events)?;
        Ok(())
    }

    pub fn mouse_up(&self) -> Result<(), std::io::Error> {   

        let events = [
            InputEvent::from(
                KeyEvent::new(
                    VirtualTrackpad::ZERO, 
                    Key::ButtonLeft, 
                    KeyState::pressed(false))
                ).into_raw(),
            InputEvent::from(
                SynchronizeEvent::new(
                    VirtualTrackpad::ZERO, 
                    SynchronizeKind::Report, 
                    0)
                ).into_raw(),
        ];

        self.handle.write(&events)?;
        Ok(())
    }

    // delay is in milliseconds
    pub fn mouse_up_delay(&self, delay: Duration) -> Result<(), std::io::Error> {
        
        thread::sleep(delay);

        let events = [
            InputEvent::from(
                KeyEvent::new(
                    VirtualTrackpad::ZERO, 
                    Key::ButtonLeft, 
                    KeyState::pressed(false))
                ).into_raw(),
            InputEvent::from(
                SynchronizeEvent::new(
                    VirtualTrackpad::ZERO, 
                    SynchronizeKind::Report, 
                    0)
                ).into_raw(),
        ];
        self.handle.write(&events)?;
        Ok(())
    }

    pub fn mouse_move_relative(&self, x_rel: f64, y_rel:f64) -> Result<(), std::io::Error> {
        
        // RelativeEvent::new() can only take integers, 
        // so some precision must be lost. But this needs to be done 
        // without bias, since x_rel and y_rel can be negative:
        // so we truncate the values down (floor()) if they are positive,
        // and truncate them up (ceil()) if they are negative.
        // That way, they are truncated toward 0 regardless.
        // 
        // Why does this matter? Because it prevents the effect of the 
        // origin (from which relative motion is calculated) seeming to 
        // drift up or down the trackpad instead of staying where the 
        // three finger drag started.
        let x_rel_int = if x_rel > 0.0 {
            x_rel.floor() as i32
        } else {
            x_rel.ceil() as i32
        };

        let y_rel_int = if y_rel > 0.0 {
            y_rel.floor() as i32
        } else {
            y_rel.ceil() as i32
        };

        let events = [
            InputEvent::from(
                RelativeEvent::new(
                    VirtualTrackpad::ZERO, 
                    RelativeAxis::X, 
                    x_rel_int)
                ).into_raw(),
            InputEvent::from(
                RelativeEvent::new(
                    VirtualTrackpad::ZERO, 
                    RelativeAxis::Y, 
                    y_rel_int)
                ).into_raw(),
            InputEvent::from(
                SynchronizeEvent::new(
                    VirtualTrackpad::ZERO, 
                    SynchronizeKind::Report, 
                    0)
                ).into_raw(),
        ];
        self.handle.write(&events)?;
        Ok(())
    }

    pub fn destruct(self) -> Result<(), std::io::Error>{
        self.handle.dev_destroy()
    }
}