//! # Threaded Proxy Logger
//!
//! This module provides a `ThreadedProxyLogger` struct that implements the `log::Log` trait.
//!
//! The `ThreadedProxyLogger` forwards log messages to other loggers, but it does so in a separate thread.
//! This can be useful in scenarios where logging can be a bottleneck, for example, when the logger writes to a slow output (like a file or a network), or when there are a lot of log messages.
//!
//! As an async framework might not yet be available when the logger is set up, async/await is not used.
//!
//! ## Usage
//!
//! First, create the actual logger that you want to use, like for example `TermLogger` or `CombinedLogger` from the `simplelog` crate.<BR>
//! Then, initialize the `ThreadedProxyLogger` with the maximum log level and the proxied logger:
//!
//! ```rust
//! use log::LevelFilter;
//! use simplelog::TermLogger;
//! use threaded_proxy_logger::ThreadedProxyLogger;
//!
//! let proxied_logger = TermLogger::new(
//!     LevelFilter::Info,
//!     simplelog::Config::default(),
//!     simplelog::TerminalMode::Mixed,
//!     simplelog::ColorChoice::Auto
//! );
//! // or alternatively combine multiple loggers with CombinedLogger or alike
//!
//! let result = ThreadedProxyLogger::init(LevelFilter::Info, proxied_logger);
//! assert!(result.is_ok());
//! ```
//!
//! Now, you can use the `log` crate's macros (`error!`, `warn!`, `info!`, `debug!`, `trace!`) to log messages.
//! The `ThreadedProxyLogger` will forward these messages to the proxied loggers in a separate thread.
//!

use std::{
    sync::mpsc::{Receiver, Sender},
    thread::{self, JoinHandle},
};

use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

/// A custom representation of the `log::Record` struct which is unfortunately
/// not directly serializable (mostly due to the use of Arguments).
/// Used to send data through the channel.
struct RecordMsg {
    level: Level,
    args: String,
    module_path: Option<String>,
    target: String,
    file: Option<String>,
    line: Option<u32>,
}

impl RecordMsg {
    const fn new(
        level: Level,
        args: String,
        module_path: Option<String>,
        target: String,
        file: Option<String>,
        line: Option<u32>,
    ) -> Self {
        Self {
            level,
            args,
            module_path,
            target,
            file,
            line,
        }
    }
}

/// The types of message that can be sent through the channel
enum MsgType {
    Data(RecordMsg),
    Flush,
    Shutdown,
}

/// A `log::Log` implementation that executes all logging on a separate thread.<p>
/// Simply pass the actual logger in the call to `ThreadedProxyLogger::init`. To pass multiple loggers, pass
/// an implementation like `simplelog::CombinedLogger` that bundles multiple loggers in one.</p>
pub struct ThreadedProxyLogger {
    tx: Sender<MsgType>,
    log_level: LevelFilter,
    join_handle: Option<JoinHandle<()>>,
}

impl ThreadedProxyLogger {
    /// Initializes the `ThreadedProxyLogger`.
    ///
    /// This function sets up a new `ThreadedProxyLogger` with the specified log level and a proxied logger.
    /// It starts a new logging thread that listens for log messages on a channel.
    /// The `ThreadedProxyLogger` is then set as the logger for the `log` crate.
    ///
    /// # Arguments
    ///
    /// * `log_level` - The maximum log level that the logger will handle. Log messages with a level
    ///   higher than this will be ignored. This will also apply to the proxied logger even though it might have a higher log level set in its config.
    /// * `proxied_logger` - The actual logger that the `ThreadedProxyLogger` will forward log messages to. Use a logger like `simplelog::CombinedLogger` to log to multiple loggers.
    ///
    /// # Returns
    ///
    /// If successful, this function returns `Ok(())`. If another logger was already set for the `log` crate,
    /// this function returns `Err(SetLoggerError)`.
    ///
    /// # Errors
    ///
    /// See above
    pub fn init(log_level: LevelFilter, proxied_logger: Box<dyn Log>) -> Result<(), SetLoggerError> {
        let (tx, rx) = std::sync::mpsc::channel();
        let join_handle = Self::start_thread(rx, proxied_logger);

        let tpl = Self {
            tx,
            log_level,
            join_handle: Some(join_handle),
        };

        log::set_boxed_logger(Box::new(tpl))?;
        log::set_max_level(log_level);
        Ok(())
    }

    /// Starts the thread that listens to incoming log events
    fn start_thread(rx: Receiver<MsgType>, proxied_logger: Box<dyn Log>) -> JoinHandle<()> {
        thread::spawn(move || {
            while let Ok(message) = rx.recv() {
                match message {
                    MsgType::Data(message) => Self::log_record(&message, &proxied_logger),
                    MsgType::Flush => proxied_logger.flush(),
                    MsgType::Shutdown => break,
                };
            }
        })
    }

    /// Logs the passed log record with the registered proxied logger
    fn log_record(message: &RecordMsg, proxied_logger: &dyn Log) {
        let mut builder = Record::builder();
        proxied_logger.log(
            // this has to be done inline like this because otherwise format_args! will complain
            &builder
                .level(message.level)
                .args(format_args!("{}", message.args))
                .module_path(message.module_path.as_deref())
                .target(message.target.as_str())
                .file(message.file.as_deref())
                .line(message.line)
                .build(),
        );
    }

    fn send(&self, msg: MsgType) {
        if let Err(e) = self.tx.send(msg) {
            eprintln!("An internal error occurred in ThreadedProxyLogger: {e}");
        }
    }
}

impl Log for ThreadedProxyLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.log_level
    }

    fn log(&self, record: &Record) {
        // Converts the log::Record struct into the custom struct
        let message: RecordMsg = RecordMsg::new(
            record.level(),
            record.args().to_string(),
            record.module_path().map(str::to_owned),
            record.target().to_owned(),
            record.file().map(str::to_owned),
            record.line(),
        );

        self.send(MsgType::Data(message));
    }

    fn flush(&self) {
        // Flushing is forwarded to the actual logger
        self.send(MsgType::Flush);
    }
}

impl Drop for ThreadedProxyLogger {
    /// Sends a shutdown signal to the started thread and waits for the thread
    /// to finish processing all log messages still in the queue.
    fn drop(&mut self) {
        self.send(MsgType::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            if let Err(e) = join_handle.join() {
                eprintln!("An internal error occurred while shutting down ThreadedProxyLogger: {e:?}");
            }
        }
        // Let's allow it to be None
    }
}
