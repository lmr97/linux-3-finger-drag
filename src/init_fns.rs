pub mod config {

    use serde::Deserialize;
    use serde_json::from_str;
    use std::fs::read_to_string;
    use std::path::PathBuf;

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Configuration {
        pub acceleration: f64,
        pub drag_end_delay: u64, // in milliseconds
        pub min_motion: f64,
    }

    impl Default for Configuration {
        fn default() -> Self {
            Configuration {
                acceleration: 1.0,
                drag_end_delay: 0,
                min_motion: 0.2,
            }
        }
    }

    pub type Error = String;

    // Configs are so optional that their absence should not crash the program,
    // So if there is any issue with the JSON config file,
    // the following default values will be returned:
    //
    //      acceleration = 1.0
    //      dragEndDelay = 0
    //
    // The user is also warned about this, so they can address the issues
    // if they want to configure the way the program runs.
    pub fn parse_config_file() -> Result<Configuration, Error> {
        let config_folder = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(config_dir) => PathBuf::from(config_dir),
            None => {
                let home = std::env::var_os("HOME").ok_or_else(|| {
                    // yes, this case has in fact happened to me, so it IS worth catching
                    "$HOME is either not accessible to this program, or is not defined in your environment. \
                    What's most likely, though, is it's a permissions issue with the SystemD folder created to \
                    hold the config file or executable; did you create either using sudo?".to_owned()
                })?;
                PathBuf::from(home).join(".config")
            }
        };
        let filepath = config_folder.join("linux-3-finger-drag/3fd-config.json");
        let jsonfile = read_to_string(&filepath)
            .map_err(|_| format!("Unable to locate JSON file at {:?} ", filepath))?;

        let config = from_str::<Configuration>(&jsonfile)
            .map_err(|_| "Bad formatting found in JSON file".to_owned())?;

        Ok(config)
    }
}


pub mod libinput_init {

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
    use users::{get_user_by_uid, get_current_uid, get_user_groups};
    use ansi_term::Color::{Red, Green};

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


    fn bind_to_real_trackpad(tp_udev_name: String) -> Result<Libinput, Error> {

        let mut real_trackpad = Libinput::new_from_path(Interface);

        match real_trackpad.path_add_device(&format!("/dev/input/{tp_udev_name}")) {

            Some(real_dev) => {
                println!(
                    "\n[ {} ]: Touchpad device \"{}\" (udev path: /dev/input/{}) found and successfully loaded.", 
                    Green.paint("INFO"),
                    real_dev.name(),
                    real_dev.sysname()
                );
                Ok(real_trackpad)
            },
            None => {
                println!("\n[ {} ]: Could not load the touchpad device \
                    named `/dev/input/{tp_udev_name}`. It may also be a permissions \
                    error, but the underlying crate (input.rs) does not raise \
                    errors when a device cannot be loaded, so it's unclear. \
                    Please submit a Github issue at https://github.com/lmr97/linux-3-finger-drag/issues \
                    whether you sort this out or not, so as to help others in the \
                    same situation, and help me develop a better program. Thank \
                    you for trying it out",
                    Red.paint("ERROR")
                );
                Err(
                    Error::new(
                        ErrorKind::AddrNotAvailable, 
                        "trackpad found, could not bind"
                    )
                )
            }
        }
    }

    // Deducing the error.
    // 
    // the `input` crate does not give any errors from udev_assign_seat()
    // even on failure, so we've gotta figure it out ourselves!
    
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
    fn raise_correct_error(devices_added: i8) -> Result<Libinput, std::io::Error> {

        // define condition 1b
        let you = get_user_by_uid(get_current_uid())
            .expect(
                "\n[ ERROR ]: The user that started this program (somehow) has been removed from \
                the user database! Something strange is afoot..."
            ); // error is not colored, but this is okay to me for a rare error

        // current user will (almost) always have at least one group. If not, the process
        // will crash here, but only upon initialization (not runtime)
        let your_groups = get_user_groups(you.name(), you.primary_group_id())
            .expect(
                "\n[ ERROR ]: You are, somehow, not a part of any user groups. \
                Something strange is afoot..."
            ); // error is not colored, but this is okay to me for a rare error

        let in_input_group = your_groups
            .iter()
            .find(|group| group.name() == "input")
            .is_some();
            

        if devices_added == 0 || !in_input_group {
            println!("\n[ {} ]: This program does not have permission to access \
                /dev/input to read trackpad events, most likely because you are \
                not in the user group 'input'. Make sure you've followed \
                the instructions in Step 3 in the Manual Install section of the \
                README. If you've already done all these things, try logging out \
                and logging in again. And if that doesn't help, try rebooting \
                (this can be necessary to update permissions and user groups). \
                If all of these fail, please submit a Github issue at \
                https://github.com/lmr97/linux-3-finger-drag/issues and I will \
                look into it as soon as possible.",
                Red.paint("ERROR")
            );

            return Err(
                Error::new(ErrorKind::PermissionDenied,
                    "not in user group 'input'"
                )
            );
        }

        println!("\n[ {} ]: This program was unable to find the trackpad on your device. \
            If you're seeing this, please submit a Github issue at \
            https://github.com/lmr97/linux-3-finger-drag/issues \
            and I will look into it as soon as possible. Please include the following \
            number in the bug report: dev_added_count: {}", 
            Red.paint("ERROR"),
            devices_added
        );

        Err(Error::new(ErrorKind::NotFound, "trackpad not found"))
    }
    


    pub fn find_real_trackpad() -> Result<Libinput, std::io::Error> {

        let mut all_inputs: Libinput = Libinput::new_with_udev(Interface);
        all_inputs.udev_assign_seat("seat0").unwrap();   // will not throw an error on failure!

        // Events added are dropped by the find() in the next statement, so they need to be 
        // counted beforehand. Cloning all_inputs and finding the length of the collected Vec
        // gave me issues as well, so we're sticking to a more tranparent, reliable method.
        let mut dev_added_count: i8 = 0;
        
        // Libinput adds "touchpad" to the device you use for a trackpad.
        // This finds theat device among all active ones on your computer.
        let trackpad_find_opt = all_inputs.find(
            |event| {
                dev_added_count += 1;
                event.device().has_capability(Pointer) 
                && event.device().has_capability(Gesture)
                // virtual trackpad only has "pointer" capability
            }
        );
        
        let udev_name = match trackpad_find_opt {

            Some(tp_add_ev) => tp_add_ev.device().sysname().to_string(),
            None => return raise_correct_error(dev_added_count)
        };

        bind_to_real_trackpad(udev_name)
    }
}
