//! Kubernetes operator framework for Rust
//!
//! Provides abstractions for building Kubernetes operators using the `#[kubus]` derive macro
//! to implement event handlers for custom resources.

#![warn(unused)]
#![warn(rust_2018_idioms)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::cargo)]
#![warn(clippy::unwrap_used)]
#![warn(clippy::expect_used)]
#![warn(clippy::panic)]
#![warn(clippy::todo)]
#![warn(clippy::unimplemented)]
#![warn(clippy::missing_errors_doc)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::missing_safety_doc)]
#![deny(warnings)]
#![forbid(unsafe_code)]

pub use kubus_derive::*;

pub mod context;
pub mod error;
pub mod event_handler;
pub mod finalizer;
pub mod operator;
pub mod scope;

pub use context::*;
pub use error::*;
pub use event_handler::*;
pub use finalizer::*;
pub use operator::*;
pub use scope::*;
