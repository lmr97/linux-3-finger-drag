use serde::Deserialize;
use serde_json::from_str;
use serde_with::serde_as;
use std::{
    fs::{OpenOptions, read_to_string},
    io::ErrorKind,
    path::PathBuf,
    time::Duration
};
use log::SetLoggerError;
use simplelog::{LevelFilter, SimpleLogger, WriteLogger};

// This is simply a wrapper to allow deserialization of the
// logLevel field into a simplelog::LevelFilter, albeit in
// a roundabout way.
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


#[serde_with::serde_as]  // this has to be before the #[derive]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    #[serde(default = "default_1")]
    pub acceleration: f64,

    #[serde(default = "default_0ms")]
    #[serde_as(as = "serde_with::DurationMilliSeconds<u64>")]
    pub drag_end_delay: Duration,       // in milliseconds

    #[serde(default = "default_pt_two")]
    pub min_motion: f64,

    #[serde(default = "default_5ms")]
    #[serde_as(as = "serde_with::DurationMilliSeconds<u64>")]
    pub response_time: Duration,        // in milliseconds

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
            drag_end_delay: Duration::from_millis(0),
            min_motion: 0.2,
            response_time: Duration::from_millis(5),
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
fn default_1()      -> f64      { 1.0 }
fn default_0ms()    -> Duration { Duration::from_millis(0) }
fn default_5ms()    -> Duration { Duration::from_millis(5) }
fn default_pt_two() -> f64      { 0.2 }
fn default_false()  -> bool     { false }
fn default_stdout() -> String   { "stdout".to_string() }
fn default_info()   -> LogLevel { LogLevel::Info }


// Configs are so optional that their absence should not crash the program,
// So if there is any issue with the JSON config file,
// the following default values will be returned:
//
// {
//     acceleration: 1.0,
//     dragEndDelay: 0,
//     minMotion: 0.2,
//     responseTime: 5,
//     failFast: false,
//     logFile: "stdout",
//     logLevel: "info",
// }
//
// The user is also warned about this, so they can address the issues
// if they want to configure the way the program runs.
pub fn parse_config_file() -> Result<Configuration, std::io::Error> {
    let config_folder = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(config_dir) => PathBuf::from(config_dir),
        None => {
            // yes, this case has in fact happened to me, so it IS worth catching
            if let Some(home) = std::env::var_os("HOME") {
                PathBuf::from(home).join(".config")
            } else {
                return Err(
                    std::io::Error::new(
                        ErrorKind::NotFound, 
                        "Neither $XDG_CONFIG_HOME or $HOME defined in environment"
                    )
                );
            }
        }
    };
    let filepath = config_folder.join("linux-3-finger-drag/3fd-config.json");
    let jsonfile = read_to_string(&filepath)
        .map_err(|_| 
            // more descriptive error
            std::io::Error::new(
                ErrorKind::NotFound, 
                format!("Unable to locate JSON file at {:?} ", filepath)
            )
        )?;

    // use serde's error as is
    let config = from_str::<Configuration>(&jsonfile)?;

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