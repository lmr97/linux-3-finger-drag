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
                    What's most likely, though, is it's a permissions issue with the SystemD folder created to\
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
    use input::{Libinput, LibinputInterface, event::EventTrait};

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


    pub fn find_real_trackpad() -> Result<Libinput, std::io::Error> {

        let mut all_inputs: Libinput = Libinput::new_with_udev(Interface);
        all_inputs.udev_assign_seat("seat0").unwrap();


        // Libinput adds "touchpad" to the device you use for a trackpad.
        // This finds theat device among all active ones on your computer.
        let trackpad_find_opt = all_inputs.find(
            |event| event.device().name().to_lowercase().contains("touchpad")
        );
        
        let udev_name = match trackpad_find_opt {

            Some(tp_add_ev) => tp_add_ev.device().sysname().to_string(),
            None => panic!("ERROR: there does not seem to be a trackpad on your \
                device (a device with 'touchpad' in its libinput name). If you're \
                seeing this, please submit a Github issue at \
                https://github.com/lmr97/linux-3-finger-drag/issues and I will look \
                into it as soon as possible.")
        };

        let mut real_trackpad = Libinput::new_from_path(Interface);

        match real_trackpad.path_add_device(&format!("/dev/input/{udev_name}")) {

            Some(real_dev) => {
                println!(
                    "Touchpad device \"{}\" (udev path: /dev/input/{}) found and successfully loaded.", 
                    real_dev.name(),
                    real_dev.sysname()
                );
                Ok(real_trackpad)
            },
            None => Err(
                Error::new(
                    ErrorKind::NotFound, 
                    format!("\nERROR: Could not load the touchpad device \
                    named `/dev/input/{udev_name}`. It may also be a permissions \
                    error, but the underlying crate (input.rs) does not raise \
                    errors when a device cannot be loaded, so it's unclear. \
                    Please submit a Github issue at https://github.com/lmr97/linux-3-finger-drag/issues \
                    whether you sort this out or not, so as to help others in the \
                    same situation, and help me develop a better program. Thank \
                    you for trying it out!")
                )
            )
        }
    }
}
