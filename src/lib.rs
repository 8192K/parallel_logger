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
//! Then, initialize the `ParallelLogger` with the maximum log level, the parallel execution mode, and the actual loggers:
//!
//! ```rust
//! use log::LevelFilter;
//! use simplelog::TermLogger;
//! use parallel_logger::{ ParallelLogger, ParallelMode };
//!
//! let term_logger = TermLogger::new(
//!     LevelFilter::Info,
//!     simplelog::Config::default(),
//!     simplelog::TerminalMode::Mixed,
//!     simplelog::ColorChoice::Auto
//! );
//! // create more loggers here if needed and add them to the vector below
//!
//! ParallelLogger::init(LevelFilter::Info, ParallelMode::Sequential, vec![term_logger]);
//! ```
//! `ParallelMode::Sequential` will execute all actual loggers in sequence on a separate thread,
//! while `ParallelMode::Parallel` will execute each actual logger in its own thread.
//! Unless there is really high contention on the loggers, `ParallelMode::Sequental` will be sufficient for most use cases.
//!
//! Now, you can use the `log` crate's macros (`error!`, `warn!`, `info!`, `debug!`, `trace!`) to log messages.
//! The `ParallelLogger` will forward these messages to the actual loggers as configured .
//!

use std::thread::{self, JoinHandle};

use flume::{self, Receiver, Sender};
use log::{LevelFilter, Log, Metadata, Record};
use serializable_log_record::{into_log_record, SerializableLogRecord};

/// The types of message that can be sent through the channel
enum MsgType {
    Data(SerializableLogRecord),
    Flush,
    Shutdown,
}

/// The mode in which the logger will process the actual loggers
#[derive(Debug, Copy, Clone)]
pub enum ParallelMode {
    /// The logger will use a single thread to process all actual loggers in sequence
    Sequential,
    /// The logger will use a separate thread for each actual logger
    Parallel,
}

#[derive(Debug)]
/// A `log::Log` implementation that executes all logging in parallel.<BR>
/// Simply pass the actual loggers in the call to `ParallelLogger::init`.
pub struct ParallelLogger {
    tx: Sender<MsgType>,
    log_level: LevelFilter,
}

/// A handle that can be used to shut down the `ParallelLogger`
pub struct ShutdownHandle {
    tx: Sender<MsgType>,
    join_handles: Vec<JoinHandle<()>>,
}

impl ShutdownHandle {
    /// Shuts down the `ParallelLogger` and waits for all threads to finish.
    pub fn shutdown(self) -> i32 {
        if let Err(e) = self.tx.send(MsgType::Shutdown) {
            eprintln!("An internal error occurred in ParallelLogger: {e}");
            return 0;
        }
        let mut successful_joins = 0;
        for join_handle in self.join_handles {
            if let Err(e) = join_handle.join() {
                eprintln!("An internal error occurred while shutting down ParallelLogger: {e:?}");
            } else {
                successful_joins += 1;
            }
        }
        successful_joins
    }
}

impl ParallelLogger {
    /// Initializes the `ParallelLogger`.
    ///
    /// This function sets up a new `ParallelLogger` with the specified log level, the parallel execution mode and the actual loggers.
    /// Depending on the `ParallelMode` setting, either one or more threads will be created to process the actual loggers.
    /// The `ParallelLogger` is then set as the global logger for the `log` crate.
    ///
    /// # Arguments
    ///
    /// * `log_level` - The maximum log level that the logger will handle. Log messages with a level
    ///   higher than this will be ignored. This will also apply to the actual loggers even though those might
    ///   have a higher log level set in their configs.
    /// * `mode` - The parallel execution mode that the `ParallelLogger` will use.
    /// * `actual_loggers` - The actual loggers that the `ParallelLogger` will forward log messages to.
    ///
    /// # Returns
    ///
    /// A `ShutdownHandle` that can be used to shut down the `ParallelLogger` and possibly flush the remaining messages in the queue.
    ///
    /// # Panics
    ///
    /// - if another logger was already set for the `log` crate
    /// - if no actual logger was provided
    /// - if a thread could not be created
    pub fn init(log_level: LevelFilter, mode: ParallelMode, actual_loggers: Vec<Box<dyn Log>>) -> ShutdownHandle {
        assert!(
            !actual_loggers.is_empty(),
            "Failed to initialize ParallelLogger: No actual loggers provided"
        );

        let (tx, rx) = flume::unbounded();

        let mut join_handles = Vec::with_capacity(actual_loggers.len());
        match mode {
            ParallelMode::Sequential => {
                let join_handle = Self::start_thread(rx, actual_loggers);
                join_handles.push(join_handle);
            }
            ParallelMode::Parallel => {
                let mut counter = 0;

                for logger in actual_loggers {
                    let join_handle = Self::start_thread_single(rx.clone(), logger, counter);
                    join_handles.push(join_handle);
                    counter += 1;
                }
            }
        };

        let tpl = Self {
            tx: tx.clone(),
            log_level,
        };

        log::set_boxed_logger(Box::new(tpl)).unwrap();
        log::set_max_level(log_level);

        ShutdownHandle { tx, join_handles }
    }

    fn start_thread_single(rx: Receiver<MsgType>, actual_logger: Box<dyn Log>, counter: i32) -> JoinHandle<()> {
        thread::Builder::new()
            .name(format!("ParallelLogger-Thread-{counter}"))
            .spawn(move || {
                while let Ok(message) = rx.recv() {
                    match message {
                        MsgType::Data(message) => Self::log_record(&message, &*actual_logger),
                        MsgType::Flush => actual_logger.flush(),
                        MsgType::Shutdown => break,
                    };
                }
            })
            .unwrap()
    }

    /// Starts the thread that listens to incoming log events
    fn start_thread(rx: Receiver<MsgType>, actual_loggers: Vec<Box<dyn Log>>) -> JoinHandle<()> {
        thread::Builder::new()
            .name("ParallelLogger-Thread-0".to_owned())
            .spawn(move || {
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
            .unwrap()
    }

    /// Logs the passed log record with the registered actual loggers
    fn log_record(message: &SerializableLogRecord, actual_logger: &dyn Log) {
        let mut builder = Record::builder();
        // this has to be done inline like this because otherwise format_args! will complain
        actual_logger.log(&into_log_record!(builder, message));
    }

    fn send(&self, msg: MsgType) {
        if let Err(e) = self.tx.send(msg) {
            eprintln!("An internal error occurred in ParallelLogger: {e}");
        }
    }
}

impl Log for ParallelLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.log_level
    }

    /// Forwards the log call to the actual loggers
    fn log(&self, record: &Record) {
        // Converts the log::Record struct into the custom struct
        self.send(MsgType::Data(SerializableLogRecord::from(record)));
    }

    /// Forwards the flush call to the actual loggers
    fn flush(&self) {
        // Flushing is forwarded to the actual loggers
        self.send(MsgType::Flush);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use log::{LevelFilter, Log, Metadata, Record};
    use std::{sync::mpsc::Sender, time::Duration};

    struct ChannelLogger {
        level: LevelFilter,
        sender: Sender<SerializableLogRecord>,
    }

    impl ChannelLogger {
        pub fn new(level: LevelFilter, sender: Sender<SerializableLogRecord>) -> Box<Self> {
            Box::new(Self { level, sender })
        }
    }

    impl Log for ChannelLogger {
        fn enabled(&self, metadata: &Metadata) -> bool {
            metadata.level() <= self.level
        }

        fn log(&self, record: &Record) {
            if self.enabled(record.metadata()) {
                let msg = SerializableLogRecord::from(record);
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
        let (tx3, rx3) = std::sync::mpsc::channel();

        let logger = ChannelLogger::new(LevelFilter::Info, tx);
        let logger2 = ChannelLogger::new(LevelFilter::Info, tx2);
        let logger3 = ChannelLogger::new(LevelFilter::Error, tx3);

        // due to the log crate working across single unit tests, we can only call init once...
        let shutdown_handle = ParallelLogger::init(LevelFilter::Info, ParallelMode::Sequential, vec![logger, logger2, logger3]);

        log::info!("Test message");
        let msg = rx.recv_timeout(Duration::from_secs(2));
        assert!(msg.is_ok());

        let msg = msg.unwrap();
        assert_eq!(msg.level, "INFO");
        assert_eq!(msg.args, "Test message");
        assert_eq!(msg.module_path, Some("parallel_logger::test".into()));
        assert_eq!(msg.target, "parallel_logger::test");
        assert_eq!(msg.file, Some("src/lib.rs".to_owned()));
        assert!(msg.line.is_some());

        let msg = rx2.recv_timeout(Duration::from_secs(2));
        assert!(msg.is_ok());

        let msg = msg.unwrap();
        assert_eq!(msg.level, "INFO");
        assert_eq!(msg.args, "Test message");
        assert_eq!(msg.module_path, Some("parallel_logger::test".into()));
        assert_eq!(msg.target, "parallel_logger::test");
        assert_eq!(msg.file, Some("src/lib.rs".to_owned()));
        assert!(msg.line.is_some());

        assert!(rx3.recv_timeout(Duration::from_secs(2)).is_err());

        assert_eq!(shutdown_handle.shutdown(), 1);
    }

    #[test]
    #[should_panic]
    fn test_parallel_logger_no_actual_loggers() {
        ParallelLogger::init(LevelFilter::Info, ParallelMode::Sequential, vec![]);
    }
}
