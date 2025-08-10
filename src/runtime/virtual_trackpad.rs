// this file is basically copied and rearranged from arcnmx's GitHub example
// in the input-linux-rs repo (a translation of an example
// on the Linux kernel's uinput module, actually). 
// The Rust example can be found here: 
// https://github.com/arcnmx/input-linux-rs/blob/main/examples/mouse-movements.rs

use std::{
    fs::{File, OpenOptions}, 
    os::{fd::AsFd, unix::fs::OpenOptionsExt}, 
    thread, time::{self, Duration}
};
use smol::{
    channel::{Receiver, RecvError}, 
    future::FutureExt
};
use input_linux::{
    EventKind, EventTime, 
    InputEvent, InputId, 
    Key, KeyEvent, KeyState, 
    RelativeAxis, RelativeEvent, 
    SynchronizeEvent, SynchronizeKind, 
    UInputHandle
};

use nix::libc::O_NONBLOCK;
use tracing::{debug, error, trace};

use crate::runtime::event_handler::ControlSignal::{self, *};

pub enum VtpError {
    EventWriteError(std::io::Error), 
    ChannelRecvError(RecvError),
    IntConversionError(<i64 as TryInto<i64>>::Error)
}

impl From<std::io::Error> for VtpError {
    fn from(err: std::io::Error) -> Self {
        VtpError::EventWriteError(err)
    }
}

impl From<RecvError> for VtpError {
    fn from(err: RecvError) -> Self {
        VtpError::ChannelRecvError(err)
    }
}


/// This struct is stateless: no position or mouse state is available.
/// This is due to issues that arise from mutability in a mulit-thread
/// context. If state is required, a wrapper struct will need to be
/// created, or tracked externally somehow. 
pub struct VirtualTrackpad {
    handle: UInputHandle<File>,
    rx: Receiver<ControlSignal>,
    pub mouse_is_down: bool
}


// Move receiver into start_handler, keep it in the struct by reference,
// so it can be cloned
pub fn start_handler(rx: Receiver<ControlSignal>) -> Result<VirtualTrackpad, std::io::Error> {
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

    Ok(
        VirtualTrackpad { 
            handle: uhandle, 
            rx,//: Arc::new(rx), 
            mouse_is_down: false
        }
    )

}


/// Start a timer for `delay`.
/// 
/// The messy return type is to match `listen_for_signal`. It is infallible.
async fn plain_timeout(delay: Duration) -> Result<Option<ControlSignal>, RecvError>{
    trace!("Starting delay of {:?}", delay);
    smol::Timer::after(delay).await;
    trace!("Delay completed fully");
    Ok(None)
}

impl Clone for VirtualTrackpad {
    /// This clone() can theoretically panic since there is an expect() in 
    /// its definition. This is because `try_cloned_to_owned`, from `std::io`,
    /// utilizes libc's `fnctl`, which can fail, but will only do so if the 
    /// duplicating the file descriptor would exceed the maximum number of 
    /// file descriptors to be opened (or if the arguments to it are invalid, 
    /// but the Rust method takes no arguments except for a known-valid FD, 
    /// so those arguments are controlled by the std library).
    /// 
    /// This makes it as safe as any other file-system function to call, since 
    /// it only fails when there is a resource limitation issue (which would be 
    /// a rare and system-wide problem).
    /// 
    /// Note that the boolean `mouse_is_down` is *copied*, **not** passed by 
    /// reference, for simplicity. 
    fn clone(&self) -> Self {
        let uinput_fd = self.handle
            .as_fd()
            .try_clone_to_owned()
            .expect(
                "uinput file descriptor could not be duplicated, \
                likely do to hitting the maximum open file descriptors \
                for this OS."
        );

        VirtualTrackpad {
            handle: UInputHandle::new(File::from(uinput_fd)),
            rx: self.rx.clone(),
            mouse_is_down: self.mouse_is_down
        }
    }
}


impl VirtualTrackpad
{
    const ZERO: EventTime = EventTime::new(0, 0);

    pub fn mouse_down(&mut self) -> Result<(), std::io::Error> {
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
        self.mouse_is_down = true;
        Ok(())
    }

    pub fn mouse_up(&mut self) -> Result<(), VtpError> {   

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
        self.mouse_is_down = false;

        Ok(())
    }


    /// This is an infinite loop that listens for and processes signals
    /// for a delay to the end of the drag, like cancelation. This 
    /// thread will not panic, and will not stop unless either it's 
    /// sent a `ControlSignal::TerminateThread`, or an error was 
    /// raised. So if it ends prematurely, it's because of an error.
    pub async fn handle_mouse_up_timeout(&mut self, delay: Duration) -> Result<(), VtpError> {
        
        loop {
            trace!("starting new loop of handle_mouse_up_timeout");
            let ctl_sig = self.rx.recv().await?;
            debug!("sig recv'd in outer loop: {:?}", ctl_sig);

            // handle signals received during outer loop
            match ctl_sig {
                RestartTimer  => {},        // proceed to timer
                CancelTimer => {
                    trace!("Setting mouse up now");
                    self.mouse_up()?;
                    continue;
                },
                CancelMouseUp => continue,  // don't do anything this iteration
                TerminateThread => break
            }

            // handle signals received during timer loop
            // that can't be handled within that scope
            match self.run_timer(delay).await? {
                Some(signal) => {
                    match signal {
                        CancelMouseUp => continue,
                        TerminateThread => break,
                        _ => {}                     // cancel/restart timer have already been handled
                    }
                },
                None => {}
            }

            self.mouse_up()?;
            debug!("mouse_up written from async mouse_up fn");
        }

        Ok(())
    }

    
    /// A simple, blocking mouse_up, but with a set, blocking, uncancellable delay. 
    /// `delay` is measured in milliseconds.
    pub fn mouse_up_delay_blocking(&mut self, delay: Duration) -> Result<(), VtpError> {
        
        std::thread::sleep(delay);

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

        debug!("mouse_up written from mouse_up_delay_blocking");

        self.mouse_is_down = false;
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

    
    // dragEndDelay time should be cut short by a pointer or scoll gesture;
    // this function listens on the channel for either a pointer button press,
    // a scoll event, or a non-3-finger gesture, and exits when it gets one
    async fn listen_for_signal(&self) -> Result<Option<ControlSignal>, RecvError> {

        // function blocks until signal is received
        // since ControlSignal is the only thing
        // ever sent in the channel, there's no need to
        // check that that's what we received
        debug!("Listening for cancel signal...");
        trace!("Size of buffer currently: {}", self.rx.len());
        let sig = self.rx.recv().await?;
        debug!("Signal received: {:?}", sig);
        Ok(Some(sig))
    }


    /// A timer that can be cancelled or reset via a signal in the channel. The return value
    /// is what signal was received, if any, so they 
    async fn run_timer(&self, delay: Duration) -> Result<Option<ControlSignal>, RecvError>{
        loop {
            // returns the output of the function that returns first
            let signal_opt = plain_timeout(delay)
                .or(self.listen_for_signal())
                .await?;
            
            match signal_opt {
                Some(signal) => {
                    match signal {
                        RestartTimer => continue,  
                        // function exits, lets the outer loop handle the other signals
                        _ => return Ok(Some(signal)), 
                    }
                },
                None => break
            }
        }
        Ok(None)
    }

    pub fn destruct(self) -> Result<(), std::io::Error> {
        self.handle.dev_destroy()
    }
}