//! Kubernetes operator framework for Rust
//!
//! Kubus provides a `#[kubus]` macro for building Kubernetes operators with minimal boilerplate.
//! It wraps [kube-rs](https://kube.rs) controllers with an ergonomic function-based API.
//!
//! # Examples
//!
//! ## Basic operator
//!
//! ```rust,no_run
//! use std::{sync::Arc, time::Duration};
//! use k8s_openapi::api::core::v1::Pod;
//! use kube::{Client, ResourceExt};
//! use kubus::{Context, HandlerError, Operator, kubus};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), kubus::Error> {
//!     let client = Client::try_default().await?;
//!
//!     Operator::builder()
//!         .with_context(client)
//!         .handler(on_pod)
//!         .run()
//!         .await
//! }
//!
//! #[kubus(event = Apply)]
//! async fn on_pod(pod: Arc<Pod>, _ctx: Arc<Context>) -> Result<(), HandlerError> {
//!     println!("Pod {} in namespace {}",
//!         pod.name_unchecked(),
//!         pod.namespace().unwrap()
//!     );
//!     Ok(())
//! }
//! ```
//!
//! ## Multiple handlers with label selectors
//!
//! ```rust,no_run
//! use std::{sync::Arc, time::Duration};
//! use k8s_openapi::api::core::v1::Pod;
//! use kube::{Client, ResourceExt};
//! use kubus::{Context, HandlerError, Operator, kubus};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), kubus::Error> {
//!     let client = Client::try_default().await?;
//!
//!     Operator::builder()
//!         .with_context(client)
//!         .handler(on_pod_apply)
//!         .handler(on_pod_delete)
//!         .run()
//!         .await
//! }
//!
//! #[kubus(
//!     event = Apply,
//!     label_selector = "app.kubernetes.io/managed-by=kubus"
//! )]
//! async fn on_pod_apply(pod: Arc<Pod>, _ctx: Arc<Context>) -> Result<(), HandlerError> {
//!     println!("Apply: {}", pod.name_unchecked());
//!     Ok(())
//! }
//!
//! #[kubus(
//!     event = Delete,
//!     label_selector = "app.kubernetes.io/managed-by=kubus"
//! )]
//! async fn on_pod_delete(pod: Arc<Pod>, _ctx: Arc<Context>) -> Result<(), HandlerError> {
//!     println!("Delete: {}", pod.name_unchecked());
//!     Ok(())
//! }
//! ```
//!
//! ## Custom state and finalizers
//!
//! ```rust,no_run
//! use std::{sync::Arc, time::Duration};
//! use k8s_openapi::api::core::v1::ConfigMap;
//! use kube::{Client, ResourceExt};
//! use kubus::{Context, HandlerError, Operator, kubus};
//!
//! #[derive(Debug, Clone)]
//! struct State {
//!     db_pool: String,
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), kubus::Error> {
//!     let client = Client::try_default().await?;
//!     let state = State { db_pool: "connection_string".to_string() };
//!
//!     Operator::builder()
//!         .with_context((client, state))
//!         .handler(on_configmap_apply)
//!         .handler(on_configmap_delete)
//!         .run()
//!         .await
//! }
//!
//! #[kubus(event = Apply, finalizer = "kubus.io/cleanup")]
//! async fn on_configmap_apply(
//!     cm: Arc<ConfigMap>,
//!     ctx: Arc<Context<State>>
//! ) -> Result<(), HandlerError> {
//!     println!("ConfigMap {} - db: {}", cm.name_unchecked(), ctx.data.db_pool);
//!     Ok(())
//! }
//!
//! #[kubus(event = Delete, finalizer = "kubus.io/cleanup")]
//! async fn on_configmap_delete(
//!     cm: Arc<ConfigMap>,
//!     _ctx: Arc<Context<State>>
//! ) -> Result<(), HandlerError> {
//!     println!("Cleanup for {}", cm.name_unchecked());
//!     Ok(())
//! }
//! ```

/// Errors that can occur during operator execution
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Error during JSON serialization/deserialization
    #[error("SerializationError: {0}")]
    SerializationError(#[source] serde_json::Error),
    /// Error from the Kubernetes client
    #[error("Kube Error: {0}")]
    KubeError(#[from] kube::Error),
    /// Error returned from the event handler
    #[error("Handler Error: {0}")]
    Handler(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Result type for Kubus operations
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[doc(inline)]
pub use kubus_derive::*;

pub mod ext;
#[doc(inline)]
pub use ext::{ApiExt, ScopeExt};

pub mod context;
#[doc(inline)]
pub use context::Context;

pub mod event_handler;
#[doc(inline)]
pub use event_handler::{EventType, Handler, HandlerError, Runnable};

pub mod operator;
#[doc(inline)]
pub use operator::Operator;

pub mod finalizer;
#[doc(inline)]
pub use finalizer::{apply_finalizer, remove_finalizer};

/// Print list of CRD's to stdout as serialized yaml
///
/// ```rust,no_run
/// print_crds![Database, Backup];
/// ```
#[macro_export]
macro_rules! print_crds {
    [$($resource:ident),+] => {{
            let list = ::k8s_openapi::List {
                items: vec![$($resource::crd(),)+],
                ..::core::default::Default::default()
            };
            let yaml = ::serde_yaml::to_string(&list).unwrap();
            let mut stdout = ::std::io::stdout();
            stdout.write_all(yaml.as_bytes())?;
    }};
}
