extern crate regex;
extern crate signal_hook;
extern crate serde;
extern crate serde_json;

use regex::Regex;
use std::io::{self, BufRead};
use std::ffi::OsString;
use std::fs::read_to_string;
use std::iter::Iterator;
use std::process::{Command, Stdio};
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::Deserialize;
use serde_json::from_str;

mod uinput_handler;

#[derive(Deserialize)]
struct Configuration {
    acceleration: f32,
    drag_end_delay: u64  // in milliseconds
}

// Configs are so optional that their absence should not crash the program,
// So if there is any issue with the JSON config file, 
// the following default values will be returned:
//
//      acceleration = 1.0
//      dragEndDelay = 0
//
// The user is also warned about this, so they can address the issues 
// if they want to configure the way the program runs.
fn parse_config_file(filepath: OsString) -> Configuration {

    let Ok(jsonfile) = read_to_string::<&OsString>(&filepath) else {
        println!("WARNING: Unable to locate JSON file at {:?}; using defaults of:

            acceleration = 1.0
            dragEndDelay = 0
            ", filepath);

        return Configuration {
            acceleration: 1.0, 
            drag_end_delay: 0
        };
    };

    let Ok(config) = from_str::<Configuration>(&jsonfile) else {
        println!("WARNING: Bad formatting found in JSON file, falling back on defaults of:
    
            acceleration = 1.0
            dragEndDelay = 0
            ");
    
        return Configuration {
            acceleration: 1.0, 
            drag_end_delay: 0
        };
    };

    config
}
 
fn main() {

    // handling SIGINT and SIGTERM
    let sigterm_received = Arc::new(AtomicBool::new(false));
    let sigint_received  = Arc::new(AtomicBool::new(false));

    signal_hook::flag::register(
        signal_hook::consts::SIGTERM, 
        Arc::clone(&sigterm_received)
    ).unwrap();

    signal_hook::flag::register(
        signal_hook::consts::SIGINT, 
        Arc::clone(&sigint_received)
    ).unwrap();
    
    // Rust does not expand ~ notation in Unix filepath strings, 
    // so we have to implement it ourselves.
    // 
    // Starting with getting $HOME...
    let configs = if let Some(home) = env::var_os("HOME") {
        let path_from_home_dir = "/.config/linux-3-finger-drag/3fd-config.json";

        let mut full_path = home;
        full_path.push(path_from_home_dir);

        parse_config_file(full_path)
    } else {
        // yes, this case has in fact happened to me, so it IS worth catching
        println!("
        $HOME is either not accessible to this program, or is not defined in your environment.
        What's most likely, though, is it's a permissions issue with the SystemD folder created to 
        hold the config file or executable; did you create either using sudo?

        The configuration file (at least) will not be accessed, and the program will continue
        execution (if possible), using defaults of:

            acceleration = 1.0
            dragEndDelay = 0
        ");

        Configuration {
            acceleration: 1.0, 
            drag_end_delay: 0
        }
    };    
    
    let mut vtrackpad = uinput_handler::start_handler();

    let output = Command::new("stdbuf")
        .arg("-o0")
        .arg("libinput")
        .arg("debug-events")
        .stdout(Stdio::piped())
        .spawn()
        .expect("You are not yet allowed to read libinput's debug events.
        Have you added yourself to the group \"input\" yet?
        (see installation section in README, step 3.2)
        ")
        .stdout
        .expect("libinput has no stdout");


    let mut xsum: f32 = 0.0;
    let mut ysum: f32 = 0.0;
    let pattern = Regex::new(r"[\s]+|/|\(").unwrap();

    for line in io::BufReader::new(output).lines() {
        
        // handle interrupts
        if sigterm_received.load(Ordering::Relaxed) || sigint_received.load(Ordering::Relaxed) {
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
            if finger != "3" && !action.starts_with("GESTURE_HOLD"){
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

    vtrackpad.mouse_up();       // just in case
    vtrackpad.dev_destroy();    // we don't need virtual devices cluttering the system

}
