# parallel_logger

[![Crates.io](https://img.shields.io/crates/v/parallel_logger.svg)](https://crates.io/crates/parallel_logger)
[![Docs](https://docs.rs/parallel_logger/badge.svg)](https://docs.rs/parallel_logger)
[![MIT/APACHE-2.0](https://img.shields.io/crates/l/parallel_logger.svg)](https://crates.io/crates/parallel_logger)

A simple logger that does not do logging by itself but passes all log events to an arbitrary number of actual loggers running in parallel. Depending on the parallel execution mode, the actual loggers are executed either in sequence (`ParallelMode::Sequential`) on one thread or in parallel (`ParallelMode::Parallel`) using one thread per actual logger. `ParallelMode::Sequential` should be sufficient for most use cases.

Very useful when logging is a bottleneck such as in realtime scenarios and/or when logging to a network or database etc.

## Usage

Add the dependency to your `Cargo.toml`:

```toml
[dependencies]
log = "0.4"
parallel_logger = "0.3"
```

How to use in your application:

```rust
use parallel_logger::{ParallelLogger, ParallelMode};

fn main() {
    ParallelLogger::init(log::LevelFilter::Info, ParallelMode::Sequential, vec![any_logger_1, any_logger_2, ...]>);
}
```
Make sure not to create other loggers by using their respective `init` methods, but to use their `new` methods instead.
Do not register any other logger with the log crate before as the ParallelLogger will take that place.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

