use std::io::{Error, ErrorKind};
use nix::libc::{O_RDONLY, O_RDWR, O_WRONLY};
use std::fs::{File, OpenOptions};
use std::os::unix::{fs::OpenOptionsExt, io::OwnedFd};
use std::path::Path;
use input::{
    Libinput, 
    LibinputInterface, 
    event::EventTrait, 
    DeviceCapability::{Gesture, Pointer}
};
use tracing::{debug, info, error};
use users::{get_user_by_uid, get_current_uid, get_user_groups};

// straight from the docs for input.rs, if I'm honest
pub struct Interface;

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        OpenOptions::new()
            .custom_flags(flags)
            .read((flags & O_RDONLY != 0) | (flags & O_RDWR != 0))
            .write((flags & O_WRONLY != 0) | (flags & O_RDWR != 0))
            .open(path)
            .map(|file| file.into())
            .map_err(|err| err.raw_os_error().unwrap())
    }
    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(File::from(fd));
    }
}


/// Add devices to `Libinput` struct. 
fn bind_to_real_trackpads(trackpads: Vec<input::Device>) -> Result<Libinput, Error> {

    let mut real_trackpads = Libinput::new_from_path(Interface);

    for tp_dev in trackpads {
        
        match real_trackpads.path_add_device(&format!("/dev/input/{}", tp_dev.sysname())) {

            Some(real_dev) => {
                info!("A touchpad found and loaded.");
                debug!("The touchpad device found: \"{}\" (udev path: /dev/input/{}).", 
                    real_dev.name(), real_dev.sysname()
                );
            },
            None => {
                error!("Could not load the touchpad device \
                    at `/dev/input/{}`. It may also be a permissions \
                    error, but the underlying crate (input.rs) does not raise \
                    errors when a device cannot be loaded, so it's unclear. \
                    Please submit a Github issue at https://github.com/lmr97/linux-3-finger-drag/issues \
                    whether you sort this out or not, so as to help others in the \
                    same situation, and help me develop a better program. Thank \
                    you for trying it out",
                    tp_dev.sysname()
                );
                return Err(
                    Error::new(
                        ErrorKind::AddrNotAvailable, 
                        "trackpad found, could not bind"
                    )
                )
            }
        }
    }

    Ok(real_trackpads)
}


/// Produce the correct error and logs to pinpoint the cause of the issue. 
fn raise_correct_error(devices_added: u8) -> Result<Libinput, std::io::Error> {

    // Since the `input` crate does not give any errors from 
    // udev_assign_seat() even on failure, we've gotta figure 
    // it out ourselves! This will not return `Ok(Libinput)` in any
    // control path; the return type is chosen only for compatibility
    // with its caller, `find_real_trackpad()`.
    //
    // 
    // There are two possible error conditions for this match arm:
    // 
    //    1. Insufficient permissions to access /dev/input
    //
    //    2. The device does not have a "trackpad" or "touchpad" 
    //       in the name given by libinput
    // 
    // Here is how I am checking for each condition:
    // 
    //    1. Permissions -- If one of the following is true:
    //       a. The program found 0 libinput events at all 
    //       b. The user is not in the 'input' group
    // 
    //    2. Not found -- no conditions from (1.) are satisfied. 
    //       This is a bug (if there's actually a trackpad), and 
    //       warrants an issue being opened on GitHub.


    // define condition 1b

    let you = match get_user_by_uid(get_current_uid()) {
        Some(user) => user,
        None => {
            error!("The user that started this program (somehow) has been removed from \
                the user database! Something strange is afoot. Exiting...");
            return Err(
                Error::new(
                    ErrorKind::PermissionDenied, 
                    "user who started the process no longer exists"
                )
            );
        }
    };

    debug!("Running user: {:?}", you);

    // current user will practically always have at least one group. If not, the process
    // will crash here, but only upon initialization (not runtime)
    let your_groups = match get_user_groups(you.name(), you.primary_group_id()) {
        Some(groups) => groups,
        None => {
            error!("You are, somehow, not a part of any user groups. \
                Something strange is afoot. Exiting...");
            return Err(
                Error::new(
                    ErrorKind::PermissionDenied, 
                    "user is not in any user groups whatsoever"
                )
            );
        }
    };

    debug!("Your user groups: {:?}", your_groups);

    let in_input_group = your_groups
        .iter()
        .find(|group| group.name() == "input")
        .is_some();
        

    if devices_added == 0 || !in_input_group {
        error!("This program does not have permission to access \
            /dev/input to read trackpad events, most likely because you are \
            not in the user group 'input'. Make sure you've followed \
            the instructions in Step 3 in the Manual Install section of the \
            README. If you've already done all these things, try logging out \
            and logging in again. And if that doesn't help, try rebooting \
            (this can be necessary to update permissions and user groups). \
            If all of these fail, please submit a Github issue at \
            https://github.com/lmr97/linux-3-finger-drag/issues and I will \
            look into it as soon as possible."
        );

        return Err(
            Error::new(ErrorKind::PermissionDenied,
                "not in user group 'input'"
            )
        );
    }

    error!("This program was unable to find the trackpad on your device. You my need \
        a reboot (or at least of user-space with `systemctl soft-reboot`) \
        to fully apply all updated permissions. If that doesn't work, please submit \
        a Github issue at https://github.com/lmr97/linux-3-finger-drag/issues \
        and I will look into it as soon as possible. Please include the following \
        number in the bug report: dev_added_count: {}",
        devices_added
    );

    Err(
        Error::new(
            ErrorKind::PermissionDenied, 
            "trackpad not discoverable by current user"
        )
    )
}


/// Find all devices that function as trackpads, returning
/// a `Libinput` struct that will receive events from all
/// trackpads.
pub fn find_real_trackpads() -> Result<Libinput, std::io::Error> {

    let mut all_inputs: Libinput = Libinput::new_with_udev(Interface);
    all_inputs.udev_assign_seat("seat0").unwrap();   // will not throw an error on failure!

    // Events added are dropped by the find() in the next statement, so they need to be 
    // counted beforehand. Cloning all_inputs and finding the length of the collected Vec
    // gave me issues as well, so we're sticking to a more tranparent, reliable method.
    let mut dev_added_count: u8 = 0;
    
    // Libinput adds "touchpad" to the device you use for a trackpad.
    // This finds theat device among all active ones on your computer.
    let all_trackpads: Vec<input::Device> = all_inputs.filter(
        |event| {
            dev_added_count += 1;
            event.device().has_capability(Pointer) 
            && event.device().has_capability(Gesture)
            // virtual trackpad only has "pointer" capability,
            // so that will not be added here
        }
    ).map(|event| event.device())
    .collect();

    if all_trackpads.len() == 0 { 
        return raise_correct_error(dev_added_count); 
    }

    bind_to_real_trackpads(all_trackpads)
}
