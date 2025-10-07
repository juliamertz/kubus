//! Kubernetes operator framework for Rust
//!
//! Provides abstractions for building Kubernetes operators using the `#[kubus]` derive macro
//! to implement event handlers for custom resources.

#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::cargo)]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![warn(unused)]
#![forbid(unsafe_code)]
#![deny(warnings)]
#![warn(clippy::unwrap_used)]
#![warn(clippy::expect_used)]
#![warn(clippy::panic)]
#![warn(clippy::todo)]
#![warn(clippy::unimplemented)]
#![warn(clippy::missing_errors_doc)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::missing_safety_doc)]

use std::{fmt::Debug, hash::Hash, str::FromStr, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::{StreamExt, future};
use k8s_openapi::{ClusterResourceScope, NamespaceResourceScope};
use kube::{
    Api, Client, Resource,
    runtime::{Controller, controller::Action, watcher::Config as ControllerConfig},
};

pub use kube;

pub use finalizer::update_finalizer;
pub use kubus_derive::kubus;
use serde::de::DeserializeOwned;

mod finalizer;

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

/// Errors that can occur during event handler execution
#[derive(thiserror::Error, Debug)]
pub enum HandlerError {
    /// Error during JSON serialization/deserialization
    #[error("SerializationError: {0}")]
    SerializationError(#[source] serde_json::Error),

    /// Error from the Kubernetes client
    #[error("Kube Error: {0}")]
    KubeError(#[from] kube::Error),

    /// Error from the Kubus
    #[error("Kubus Error: {0}")]
    KubusError(#[from] Error),
}

/// Extensions for Kubernetes resource scopes
///
/// Provides methods to create appropriate `Api` instances for namespaced
/// or cluster-scoped resources.
pub trait ScopeExt<K>
where
    K: Resource<Scope = Self>,
{
    /// Creates an API client for the resource
    ///
    /// Returns a namespaced API if namespace is provided and the resource is namespaced,
    /// otherwise returns a cluster-wide API.
    fn api(client: Client, namespace: Option<impl AsRef<str>>) -> Api<K>;
}

impl<K> ScopeExt<K> for NamespaceResourceScope
where
    K: Resource<Scope = Self>,
    K::DynamicType: Default,
{
    fn api(client: Client, namespace: Option<impl AsRef<str>>) -> Api<K> {
        if let Some(namespace) = namespace {
            Api::namespaced(client, namespace.as_ref())
        } else {
            Api::all(client)
        }
    }
}

impl<K> ScopeExt<K> for ClusterResourceScope
where
    K: Resource<Scope = Self>,
    K::DynamicType: Default,
{
    fn api(client: Client, _: Option<impl AsRef<str>>) -> Api<K> {
        Api::all(client)
    }
}

#[async_trait]
trait DynEventHandler<Err>: Send + Sync
where
    Err: std::error::Error + Send + Sync + 'static,
{
    async fn run(&self, client: Client) -> Result<(), Err>;
}

/// Handler trait for Kubernetes resource events
///
/// Implement this trait (typically via the `#[kubus]` derive macro) to define
/// custom logic for responding to resource changes.
#[async_trait]
pub trait EventHandler<K, Ctx, Err = HandlerError>
where
    K: Resource + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    Ctx: Send + Sync + 'static,
    Err: std::error::Error + Send + Sync + 'static,
{
    /// Handles a resource event
    ///
    /// Called when a resource is created, updated, or needs reconciliation.
    /// Returns an `Action` indicating when to reconcile again.
    async fn handler(resource: Arc<K>, context: Arc<Context<Ctx>>) -> Result<Action, Err>;

    /// Defines error handling policy for the handler
    ///
    /// Called when `handler` returns an error. Default implementation logs a warning
    /// and requeues after 5 seconds.
    fn error_policy(_resource: Arc<K>, err: &Err, _ctx: Arc<Context<Ctx>>) -> Action {
        tracing::warn!("Reconciliation error: {:?}", err);
        Action::requeue(Duration::from_secs(5))
    }

    /// Starts the controller watching for resource events
    ///
    /// Runs until the process receives a shutdown signal.
    async fn watch(client: Client, context: Arc<Context<Ctx>>) -> Result<(), Err>
    where
        Self: Sized + 'static,
    {
        println!("starting controller");

        let api = Api::<K>::all(client);
        Controller::new(api, ControllerConfig::default())
            .shutdown_on_signal()
            .run(Self::handler, Self::error_policy, context)
            .filter_map(|x| async move { std::result::Result::ok(x) })
            .for_each(|_| futures::future::ready(()))
            .await;

        Ok(())
    }
}

struct EventHandlerWrapper<H, K, Ctx, Err>
where
    H: EventHandler<K, Ctx, Err>,
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    Ctx: Send + Sync + 'static,
    Err: std::error::Error + Sync + Send + 'static,
{
    context: Arc<Context<Ctx>>,
    _phantom: std::marker::PhantomData<(H, K, Err)>,
}

impl<H, K, Ctx, Err> EventHandlerWrapper<H, K, Ctx, Err>
where
    H: EventHandler<K, Ctx, Err>,
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    Ctx: Send + Sync + 'static,
    Err: std::error::Error + Sync + Send + 'static,
{
    const fn new(context: Arc<Context<Ctx>>) -> Self {
        Self {
            context,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<H, K, Ctx, Err> DynEventHandler<Err> for EventHandlerWrapper<H, K, Ctx, Err>
where
    H: EventHandler<K, Ctx, Err> + Send + Sync + 'static,
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    Ctx: Send + Sync + 'static,
    Err: std::error::Error + Sync + Send + 'static,
{
    async fn run(&self, client: Client) -> Result<(), Err> {
        let context = self.context.clone();
        H::watch(client, context).await
    }
}

/// A specialized Result type for Kubus operations
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Kubernetes resource event types
#[derive(Debug, PartialEq, Eq)]
pub enum EventType {
    /// Resource created or updated
    Apply,
    /// Resource deleted
    Delete,
}

impl FromStr for EventType {
    type Err = std::io::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "Apply" => Ok(Self::Apply),
            "Delete" => Ok(Self::Delete),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no such event type",
            )),
        }
    }
}

/// Shared handler context
pub struct Context<T> {
    /// Kube client
    pub client: Client,
    /// User defined data type
    pub data: T,
}

/// Kubernetes operator managing multiple resource handlers
///
/// Use with the `#[kubus]` derive macro to register handlers for different resource types.
pub struct Operator<Ctx, Err> {
    context: Arc<Context<Ctx>>,
    handlers: Vec<Box<dyn DynEventHandler<Err>>>,
}

impl<Ctx, Err> Operator<Ctx, Err>
where
    Ctx: Send + Sync + 'static,
    Err: std::error::Error + Send + Sync + 'static,
{
    /// Creates a new operator
    pub fn new(client: Client, data: Ctx) -> Self {
        let context = Context { client, data };
        Self {
            context: Arc::new(context),
            handlers: Default::default(),
        }
    }

    /// Registers an event handler for a resource type
    ///
    /// Chain multiple calls to register handlers for different resources.
    #[must_use]
    pub fn handler<H, K>(mut self, _: H) -> Self
    where
        H: EventHandler<K, Ctx, Err> + Send + Sync + 'static,
        K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
        K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    {
        let wrapper = EventHandlerWrapper::<H, K, Ctx, Err>::new(self.context.clone());
        self.handlers.push(Box::new(wrapper));
        self
    }

    /// Runs the operator, starting all registered handlers
    ///
    /// Blocks until all handlers complete (typically on shutdown).
    pub async fn run(self) -> Result<()> {
        tracing::info!(
            "starting kubus operator with {} handlers",
            self.handlers.len()
        );

        let tasks: Vec<_> = self
            .handlers
            .into_iter()
            .map(|handler| {
                let client = self.context.client.clone();
                tokio::spawn(async move {
                    tracing::info!("starting handler");
                    let client = client.clone();
                    if let Err(e) = handler.run(client).await {
                        tracing::error!("restarting handler failed: {}", e);
                    }
                })
            })
            .collect();

        future::join_all(tasks).await;
        Ok(())
    }
}
