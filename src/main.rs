use regex::Regex;
use serde::Deserialize;
use serde_json::from_str;
use std::fs::read_to_string;
use std::io::{self, BufRead};
use std::iter::Iterator;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

mod uinput_handler;

#[derive(Deserialize, Debug)]
struct Configuration {
    acceleration: f32,
    drag_end_delay: u64, // in milliseconds
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration {
            acceleration: 1.0,
            drag_end_delay: 0,
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
fn parse_config_file() -> Result<Configuration, Error> {
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

fn main() {
    // Rust does not expand ~ notation in Unix filepath strings,
    // so we have to implement it ourselves.
    //
    // Starting with getting $HOME...
    let configs = match parse_config_file() {
        Ok(cfg) => cfg,
        Err(err) => {
            let cfg = Default::default();
            eprintln!("WARNING: {err}\n\nThe configuration file (at least) will not be accessed,\
                and the program will continue execution (if possible), using defaults of:\n {cfg:#?}");
            cfg
        }
    };

    let mut vtrackpad = uinput_handler::start_handler();

    let output = Command::new("libinput")
        .arg("debug-events")
        .stdout(Stdio::piped())
        .spawn()
        .expect(
            "You are not yet allowed to read libinput's debug events.
        Have you added yourself to the group \"input\" yet?
        (see installation section in README, step 3.2)
        ",
        )
        .stdout
        .expect("libinput has no stdout");

    let mut xsum: f32 = 0.0;
    let mut ysum: f32 = 0.0;
    let pattern = Regex::new(r"[\s]+|/|\(").unwrap();

    // handling SIGINT and SIGTERM
    let should_exit = Arc::new(AtomicBool::new(false));

    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&should_exit)).unwrap();
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&should_exit)).unwrap();

    for line in io::BufReader::new(output).lines() {
        // handle interrupts
        if should_exit.load(Ordering::Relaxed) {
            break;
        }

        let line = line.unwrap();
        if line.contains("GESTURE_") {
            // event10  GESTURE_SWIPE_UPDATE +3.769s	4  0.25/ 0.48 ( 0.95/ 1.85 unaccelerated)
            let mut parts: Vec<&str> = pattern.split(&line).filter(|c| !c.is_empty()).collect();
            let action = parts[1];
            if action == "GESTURE_SWIPE_UPDATE" && parts.len() != 9 {
                parts.remove(2);
            }
            let finger = parts[3];

            // `finger != "2"` condition prevents sticking mouse on pinch actions.
            // courtesy of Justtheflu on GitHub (issue #7)
            if finger != "3" && finger != "2" && !action.starts_with("GESTURE_HOLD") {
                // mouse_down
                vtrackpad.mouse_down();
                continue;
            }
            let cancelled = parts.len() > 4 && parts[4] == "cancelled";

            match action {
                "GESTURE_SWIPE_BEGIN" => {
                    xsum = 0.0;
                    ysum = 0.0;
                    // mouse_down
                    vtrackpad.mouse_down();
                }
                "GESTURE_SWIPE_UPDATE" => {
                    let x: f32 = parts[4].parse().unwrap();
                    let y: f32 = parts[5].parse().unwrap();
                    xsum += x * configs.acceleration;
                    ysum += y * configs.acceleration;
                    if xsum.abs() > 1.0 || ysum.abs() > 1.0 {
                        // mouse_move_relative
                        vtrackpad.mouse_move_relative(xsum, ysum);
                        xsum = 0.0;
                        ysum = 0.0;
                    }
                }
                "GESTURE_SWIPE_END" => {
                    // mouse_move_relative
                    vtrackpad.mouse_move_relative(xsum, ysum);

                    if cancelled {
                        // mouse_up
                        vtrackpad.mouse_up();
                    } else {
                        // mouse_up, with 300ms delay
                        vtrackpad.mouse_up_delay(configs.drag_end_delay);
                    }
                }
                "GESTURE_HOLD_BEGIN" => {
                    // Ignore
                }
                "GESTURE_HOLD_END" => {
                    // Ignore accidental holds when repositioning
                    if !cancelled {
                        // mouse_up
                        vtrackpad.mouse_up();
                    }
                }
                _ => {
                    // GESTURE_PINCH_*,
                    // mouse_up
                    vtrackpad.mouse_up();
                }
            }
        } else {
            // mouse_up
        }
    }

    vtrackpad.mouse_up(); // just in case
    vtrackpad.dev_destroy(); // we don't need virtual devices cluttering the system
}
