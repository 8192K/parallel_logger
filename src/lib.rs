//! # Parallel Logger
//!
//! This module provides a `ParallelLogger` struct that implements the `log::Log` trait.
//!
//! The `ParallelLogger` forwards log messages to other loggers, but it does so in a separate thread.
//! This can be useful in scenarios where logging can be a bottleneck, for example, when the logger writes
//! to a slow output (like a file or a network), or when there are a lot of log messages or in a realtime scenario.
//!
//! As an async framework might not yet be available when the logger is set up, async/await is not used.
//!
//! ## Usage
//!
//! First, create the actual loggers that you want to use, like for example `TermLogger`, `WriteLogger`
//! or even `CombinedLogger` from the `simplelog` crate. Any `log::Log` implementation will work.<BR>
//! Then, initialize the `ParallelLogger` with the maximum log level and the actual loggers:
//!
//! ```rust
//! use log::LevelFilter;
//! use simplelog::TermLogger;
//! use parallel_logger::ParallelLogger;
//!
//! let term_logger = TermLogger::new(
//!     LevelFilter::Info,
//!     simplelog::Config::default(),
//!     simplelog::TerminalMode::Mixed,
//!     simplelog::ColorChoice::Auto
//! );
//! // create more loggers here if needed and add them to the vector below
//!
//! ParallelLogger::init(LevelFilter::Info, vec![term_logger]);
//! ```
//!
//! Now, you can use the `log` crate's macros (`error!`, `warn!`, `info!`, `debug!`, `trace!`) to log messages.
//! The `ParallelLogger` will forward these messages to the actual loggers in a separate thread.
//!

use std::{
    sync::mpsc::{Receiver, Sender},
    thread::{self, JoinHandle},
};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// A custom representation of the `log::Record` struct which is unfortunately
/// not directly serializable (mostly due to the use of `Arguments`).
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
/// Simply pass the actual loggers in the call to `ParallelLogger::init`.</p>
pub struct ParallelLogger {
    tx: Sender<MsgType>,
    log_level: LevelFilter,
    join_handle: Option<JoinHandle<()>>,
}

impl ParallelLogger {
    /// Initializes the `ParallelLogger`.
    ///
    /// This function sets up a new `ParallelLogger` with the specified log level and the actual loggers.
    /// It starts a new logging thread that listens for log messages on a channel.
    /// The `ParallelLogger` is then set as the global logger for the `log` crate.
    ///
    /// # Arguments
    ///
    /// * `log_level` - The maximum log level that the logger will handle. Log messages with a level
    ///   higher than this will be ignored. This will also apply to the actual loggers even though those might
    ///   have a higher log level set in their configs.
    /// * `actual_loggers` - The actual loggers that the `ParallelLogger` will forward log messages to.
    ///
    /// # Panics
    ///
    /// If another logger was already set for the `log` crate,
    /// or if no actual logger was provided, this function panics.
    pub fn init(log_level: LevelFilter, actual_loggers: Vec<Box<dyn Log>>) {
        assert!(!actual_loggers.is_empty(), "Failed to initialize ParallelLogger: No actual loggers provided");

        let (tx, rx) = std::sync::mpsc::channel();
        let join_handle = Self::start_thread(rx, actual_loggers);

        let tpl = Self {
            tx,
            log_level,
            join_handle: Some(join_handle),
        };

        log::set_boxed_logger(Box::new(tpl)).unwrap();
        log::set_max_level(log_level);
    }

    /// Starts the thread that listens to incoming log events
    fn start_thread(rx: Receiver<MsgType>, actual_loggers: Vec<Box<dyn Log>>) -> JoinHandle<()> {
        thread::spawn(move || {
            while let Ok(message) = rx.recv() {
                match message {
                    MsgType::Data(message) => {
                        for actual_logger in &actual_loggers {
                            Self::log_record(&message, actual_logger);
                        }
                    }
                    MsgType::Flush => {
                        for actual_logger in &actual_loggers {
                            actual_logger.flush();
                        }
                    }
                    MsgType::Shutdown => break,
                };
            }
        })
    }

    /// Logs the passed log record with the registered actual loggers
    fn log_record(message: &RecordMsg, actual_logger: &dyn Log) {
        let mut builder = Record::builder();
        actual_logger.log(
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
            eprintln!("An internal error occurred in ParallelLogger: {e}");
        }
    }

    fn convert_msg(record: &Record) -> RecordMsg {
        RecordMsg::new(
            record.level(),
            record.args().to_string(),
            record.module_path().map(str::to_owned),
            record.target().to_owned(),
            record.file().map(str::to_owned),
            record.line(),
        )
    }
}

impl Log for ParallelLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.log_level
    }

    fn log(&self, record: &Record) {
        // Converts the log::Record struct into the custom struct
        self.send(MsgType::Data(Self::convert_msg(record)));
    }

    fn flush(&self) {
        // Flushing is forwarded to the actual loggers
        self.send(MsgType::Flush);
    }
}

impl Drop for ParallelLogger {
    /// Sends a shutdown signal to the started thread and waits for the thread
    /// to finish processing all log messages still in the queue.
    fn drop(&mut self) {
        self.send(MsgType::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            if let Err(e) = join_handle.join() {
                eprintln!("An internal error occurred while shutting down ParallelLogger: {e:?}");
            }
        }
        // Let's allow it to be None
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use log::{LevelFilter, Log, Metadata, Record};
    use std::{sync::mpsc::Sender, time::Duration};

    use crate::RecordMsg;

    struct ChannelLogger {
        level: LevelFilter,
        sender: Sender<RecordMsg>,
    }

    impl ChannelLogger {
        pub fn new(level: LevelFilter, sender: Sender<RecordMsg>) -> Box<Self> {
            Box::new(Self { level, sender })
        }
    }

    impl Log for ChannelLogger {
        fn enabled(&self, metadata: &Metadata) -> bool {
            metadata.level() <= self.level
        }

        fn log(&self, record: &Record) {
            if self.enabled(record.metadata()) {
                let msg = ParallelLogger::convert_msg(record);
                if self.sender.send(msg).is_err() {
                    eprintln!("Failed to send message through channel");
                }
            }
        }

        fn flush(&self) {}
    }

    #[test]
    fn test_regular_log_message() {
        let (tx, rx) = std::sync::mpsc::channel();
        let (tx2, rx2) = std::sync::mpsc::channel();

        let logger = ChannelLogger::new(LevelFilter::Info, tx);
        let logger2 = ChannelLogger::new(LevelFilter::Error, tx2);

        // due to the log crate working across single unit tests, we can only call init once...
        ParallelLogger::init(LevelFilter::Info, vec![logger, logger2]);

        log::info!("Test message");
        let msg = rx.recv_timeout(Duration::from_secs(2));
        assert!(msg.is_ok());

        let msg = msg.unwrap();
        assert_eq!(msg.level, Level::Info);
        assert_eq!(msg.args, "Test message");
        assert_eq!(msg.module_path, Some("parallel_logger::test".into()));
        assert_eq!(msg.target, "parallel_logger::test");
        assert_eq!(msg.file, Some("src/lib.rs".to_owned()));
        assert!(msg.line.is_some());

        assert!(rx2.recv_timeout(Duration::from_secs(2)).is_err());
    }

    #[test]
    #[should_panic]
    fn test_parallel_logger_no_actual_loggers() {
        ParallelLogger::init(LevelFilter::Info, vec![]);
    }
}
