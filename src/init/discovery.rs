//! Touchpad discovery by direct evdev capability inspection.
//!
//! This walks `/dev/input/event*` and checks each device for the
//! capabilities that define a multitouch touchpad -- the same criteria
//! libinput itself uses to classify one:
//!
//! * `INPUT_PROP_POINTER` (it moves a cursor, i.e. not a touchscreen)
//! * multitouch slots + positions (`ABS_MT_SLOT`, `ABS_MT_POSITION_X/Y`)
//! * `BTN_TOOL_FINGER` (finger-count reporting)
//!
//! Doing this directly (instead of going through libinput's udev seat
//! enumeration, as earlier versions did) needs no C library, no seat
//! assignment, and -- crucially -- lets us skip our *own* synthetic
//! clone, which impersonates the real touchpad's identity exactly and
//! is only distinguishable by the `phys` marker we stamp on it. That
//! matters when re-discovering after the real device re-enumerates.

use std::fs::OpenOptions;
use std::io::{Error, ErrorKind};
use std::os::unix::fs::OpenOptionsExt;

use input_linux::{AbsoluteAxis, EvdevHandle, InputProperty, Key};
use libc::O_NONBLOCK;
use tracing::{debug, error, info};

use crate::runtime::mt_proxy::CLONE_PHYS_MARKER;

/// Finds every real multitouch touchpad, returning `/dev/input/eventN`
/// paths. The caller opens these directly so it can exclusively grab
/// and proxy the raw event stream.
pub fn find_real_trackpads() -> Result<Vec<String>, Error> {
    let mut found = Vec::new();
    let mut denied = 0usize;
    let mut inspected = 0usize;

    let mut entries: Vec<_> = std::fs::read_dir("/dev/input")?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
        .map(|e| e.path())
        .collect();
    entries.sort();

    for path in entries {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                denied += 1;
                continue;
            }
            Err(_) => continue,
        };
        inspected += 1;
        let dev = EvdevHandle::new(file);

        let Ok(props) = dev.device_properties() else {
            continue;
        };
        if !props.get(InputProperty::Pointer) {
            continue; // touchscreens, keyboards, mice without the prop
        }
        let Ok(abs) = dev.absolute_bits() else {
            continue;
        };
        if !(abs.get(AbsoluteAxis::MultitouchSlot)
            && abs.get(AbsoluteAxis::MultitouchPositionX)
            && abs.get(AbsoluteAxis::MultitouchPositionY))
        {
            continue; // pointer device without multitouch (plain mouse)
        }
        let Ok(keys) = dev.key_bits() else { continue };
        if !keys.get(Key::ButtonToolFinger) {
            continue;
        }

        // Never proxy our own synthetic clone: it passes every check
        // above by design (it impersonates the real device), and is
        // recognizable only by the phys marker stamped on it.
        if let Ok(phys) = dev.physical_location() {
            if phys.starts_with(CLONE_PHYS_MARKER.as_bytes()) {
                debug!("Skipping our own synthetic clone at {}.", path.display());
                continue;
            }
        }

        let name = dev
            .device_name()
            .map(|n| {
                String::from_utf8_lossy(&n)
                    .trim_end_matches('\0')
                    .to_string()
            })
            .unwrap_or_else(|_| "<unknown>".to_string());
        info!("Touchpad found: \"{}\" at {}.", name, path.display());
        found.push(path.to_string_lossy().into_owned());
    }

    if !found.is_empty() {
        return Ok(found);
    }

    // Nothing found: produce the most useful error we can.
    if inspected == 0 && denied > 0 {
        error!(
            "This program does not have permission to read /dev/input \
            devices, most likely because you are not in the user group \
            'input'. Make sure you've followed the instructions in Step 3 \
            of the Manual Install section of the README. If you have, try \
            logging out and in again, or rebooting (group changes only \
            apply to new logins). If all of these fail, please open an \
            issue at https://github.com/lmr97/linux-3-finger-drag/issues."
        );
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "not permitted to read /dev/input (user not in group 'input'?)",
        ));
    }

    error!(
        "No multitouch touchpad was found among {} readable input \
        devices ({} unreadable). If this machine really has a touchpad, \
        please open an issue at \
        https://github.com/lmr97/linux-3-finger-drag/issues and include \
        the output of `libinput list-devices`.",
        inspected, denied
    );
    Err(Error::new(
        ErrorKind::NotFound,
        "no multitouch touchpad discoverable",
    ))
}
