use serde::Deserialize;
use serde_json::from_str;
use std::fs::{OpenOptions, read_to_string};
use std::path::PathBuf;
use log::SetLoggerError;
use simplelog::{LevelFilter, SimpleLogger, WriteLogger};

// TODO: use renames to accept lower-case
#[derive(Deserialize, Debug, Clone)]
pub enum LogLevel {
    #[serde(rename = "off")]
    Off, 
    #[serde(rename = "error")]
    Error, 
    #[serde(rename = "warn")]
    Warn, 
    #[serde(rename = "info")]
    Info, 
    #[serde(rename = "debug")]
    Debug, 
    #[serde(rename = "trace")]
    Trace
}

// we had to have a wrapper for LevelFilter for deserializing, 
// now we gotta make that wrapper useful in the program
impl Into<LevelFilter> for LogLevel {
    fn into(self) -> LevelFilter {
        match self {
            LogLevel::Off   => LevelFilter::Off,
            LogLevel::Error => LevelFilter::Error,
            LogLevel::Warn  => LevelFilter::Warn,
            LogLevel::Info  => LevelFilter::Info,
            LogLevel::Debug => LevelFilter::Debug,
            LogLevel::Trace => LevelFilter::Trace,
        }
    }
}


#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    pub acceleration: f64,
    pub drag_end_delay: u64, // in milliseconds
    pub min_motion: f64,
    #[serde(default = "default_false")]
    pub fail_fast: bool,
    #[serde(default = "default_stdout")]
    pub log_file: String,
    #[serde(default = "default_info")]
    pub log_level: LogLevel,
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration {
            acceleration: 1.0,
            drag_end_delay: 0,
            min_motion: 0.2,
            fail_fast: false,
            log_file: "stdout".to_string(),
            log_level: LogLevel::Info
        }
    }
}

// for some reason, default literals don't seem to be okay
// with the serde crate, despite several issues and PRs on the 
// subject. Using functions to yield the values is the only 
// accepted way.
fn default_false()  -> bool     { false }
fn default_stdout() -> String   { "stdout".to_string() }
fn default_info()   -> LogLevel { LogLevel::Info }

// Return errors as strings instead of full Error enums
pub type SimpleError = String;


// Configs are so optional that their absence should not crash the program,
// So if there is any issue with the JSON config file,
// the following default values will be returned:
//
// {
//     acceleration: 1.0,
//     dragEndDelay: 0,
//     minMotion: 0.2,
//     failFast: false,
//     logFile: "stdout",
//     logLevel: "info",
// }
//
// The user is also warned about this, so they can address the issues
// if they want to configure the way the program runs.
pub fn parse_config_file() -> Result<Configuration, SimpleError> {
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


pub fn init_logger(cfg: Configuration) -> Result<(), SetLoggerError>{

    println!("[PRE-LOG: INFO]: Initializing logger...");

    let log_level = cfg.log_level.into();

    // If the log file is either "stdout" or an invalid file,
    // bypass this block and go to the end, initializing a
    // SimpleLogger (for console logging)
    if cfg.log_file != "stdout" {

        match OpenOptions::new().append(true).open(&cfg.log_file) {

            Ok(log_file) => {

                WriteLogger::init(
                    log_level, 
                    simplelog::Config::default(), 
                    log_file
                )?;  
                println!(
                    "[PRE-LOG: INFO]: Logger initialized! Logging to '{}' \
                    at {}-level verbosity.", cfg.log_file, log_level
                );
                return Ok(());
            },

            Err(open_err) => println!(
                "[PRE-LOG: WARN]: Failed to open logfile '{}' due to the the following error: {}, {}.", 
                cfg.log_file,
                open_err.kind(),
                open_err
            )
            // continues on to initialize simple logger below
        };
    }

    SimpleLogger::init(
        log_level, 
        simplelog::Config::default()
    )?;
    println!(
        "[PRE-LOG: INFO]: Logger initialized! Logging to console \
        at {log_level}-level verbosity."
    ); 

    Ok(())
}