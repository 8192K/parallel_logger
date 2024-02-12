# threaded-proxy-logger

[![Crates.io](https://img.shields.io/crates/v/threaded-proxy-logger.svg)](https://crates.io/crates/threaded-proxy-logger)
[![Docs](https://docs.rs/threaded-proxy-logger/badge.svg)](https://docs.rs/threaded-proxy-logger)
[![MIT/APACHE-2.0](https://img.shields.io/crates/l/threaded-proxy-logger.svg)](https://crates.io/crates/threaded-proxy-logger)

A simple logger that does not do logging by itself but passes all log events to an arbitrary number of passed loggers which run in a separate thread.

Very useful when logging is a bottleneck such as in realtime scenarios or when logging to a network or database etc.

## Usage

Add the dependency to your `Cargo.toml`:

```toml
[dependencies]
log = "0.4"
threaded_proxy_logger = "0.5"
```

How to use in your application:

```rust
use threaded_proxy_logger::ThreadedProxyLogger;

fn main() {
    ThreadedProxyLogger::init(log::LevelFilter::Info, any_logger_that_implements_the_Log_trait>);
}
```
To pass multiple loggers, use a bundling logger like `simplelog::CombinedLogger` for example.

Make sure not to pass other loggers by using their respective `init` methods, but to use their `new` methods instead.
Do not register any other logger with the log crate before as the ThreadedProxyLogger will take that place.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

