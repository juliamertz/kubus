//! Kubernetes operator framework for Rust
//!
//! Provides abstractions for building Kubernetes operators using the `#[kubus]` derive macro
//! to implement event handlers for custom resources.

use std::error::Error as StdError;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, future};
use k8s_openapi::{ClusterResourceScope, NamespaceResourceScope};
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher::Config;
use kube::{Client, Resource, ResourceExt};
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use serde_json::json;
use thiserror::Error;

pub use kubus_derive::*;

/// Errors that can occur during operator execution
#[derive(Error, Debug)]
pub enum Error {
    /// Error during JSON serialization/deserialization
    #[error("SerializationError: {0}")]
    SerializationError(#[source] serde_json::Error),
    /// Error from the Kubernetes client
    #[error("Kube Error: {0}")]
    KubeError(#[from] kube::Error),
    /// Error returned from the event handler
    #[error("Handler Error: {0}")]
    Handler(#[source] Box<dyn StdError + Send + Sync>),
}

/// Errors that can occur during event handler execution
#[derive(Error, Debug)]
pub enum HandlerError {
    /// Error from the Kubernetes client
    #[error("Kube Error: {0}")]
    KubeError(#[from] kube::Error),
    /// Error from the Kubus
    #[error("Kubus Error: {0}")]
    KubusError(#[from] Error),
}

/// Result type for Kubus operations
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Shared handler context
#[derive(Clone)]
pub struct Context<T = ()>
where
    T: Clone,
{
    /// Kube client
    pub client: Client,
    /// User defined data type
    pub data: T,
}

impl<T> From<(Client, T)> for Context<T>
where
    T: Clone,
{
    fn from((client, data): (Client, T)) -> Self {
        Self { client, data }
    }
}

impl From<Client> for Context {
    fn from(client: Client) -> Self {
        Self { client, data: () }
    }
}

/// Kubernetes resource event types
#[derive(Debug, PartialEq, Eq)]
pub enum EventType {
    /// Resource created or updated
    Apply,
    /// Resource deleted
    Delete,
}

#[async_trait]
trait DynEventHandler<E>: Send + Sync
where
    E: StdError + Send + Sync + 'static,
{
    async fn run(&self, client: Client) -> Result<(), E>;
}

/// Handler trait for Kubernetes resource events
///
/// Implement this trait (typically via the `#[kubus]` derive macro) to define
/// custom logic for responding to resource changes.
#[async_trait]
pub trait EventHandler<K, S, E = HandlerError>
where
    K: Resource + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    S: Clone + Send + Sync + 'static,
    E: StdError + Send + Sync + 'static,
{
    const LABEL_SELECTOR: Option<&'static str> = None;

    const FIELD_SELECTOR: Option<&'static str> = None;

    /// Handles a resource event
    ///
    /// Called when a resource is created, updated, or needs reconciliation.
    /// Returns an `Action` indicating when to reconcile again.
    async fn handler(resource: Arc<K>, context: Arc<Context<S>>) -> Result<Action, E>;

    /// Defines error handling policy for the handler
    ///
    /// Called when `handler` returns an error. Default implementation logs a warning
    /// and requeues after 5 seconds.
    fn error_policy(_resource: Arc<K>, err: &E, _ctx: Arc<Context<S>>) -> Action {
        tracing::error!({ err = err as &dyn StdError }, "Handler error");

        Action::requeue(Duration::from_secs(5))
    }

    /// Starts the controller watching for resource events
    ///
    /// Runs until the process receives a shutdown signal.
    async fn watch(client: Client, context: Arc<Context<S>>) -> Result<(), E>
    where
        Self: Sized + 'static,
    {
        tracing::info!("starting controller");

        let api = Api::<K>::all(client);
        let mut config = Config::default();
        config.label_selector = Self::LABEL_SELECTOR.map(String::from);
        config.field_selector = Self::FIELD_SELECTOR.map(String::from);

        Controller::new(api, config)
            .shutdown_on_signal()
            .run(Self::handler, Self::error_policy, context)
            .filter_map(|x| async move { std::result::Result::ok(x) })
            .for_each(|_| futures::future::ready(()))
            .await;

        Ok(())
    }
}

struct EventHandlerWrapper<H, K, S, E>
where
    H: EventHandler<K, S, E>,
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    S: Clone + Send + Sync + 'static,
    E: StdError + Sync + Send + 'static,
{
    context: Arc<Context<S>>,
    _phantom: std::marker::PhantomData<(H, K, E)>,
}

impl<H, K, S, E> EventHandlerWrapper<H, K, S, E>
where
    H: EventHandler<K, S, E>,
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    S: Clone + Send + Sync + 'static,
    E: StdError + Sync + Send + 'static,
{
    const fn new(context: Arc<Context<S>>) -> Self {
        Self {
            context,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<H, K, S, E> DynEventHandler<E> for EventHandlerWrapper<H, K, S, E>
where
    H: EventHandler<K, S, E> + Send + Sync + 'static,
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    S: Clone + Send + Sync + 'static,
    E: StdError + Sync + Send + 'static,
{
    async fn run(&self, client: Client) -> Result<(), E> {
        let context = self.context.clone();
        H::watch(client, context).await
    }
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

/// Kubernetes operator managing multiple resource handlers
///
/// Use with the `#[kubus]` derive macro to register handlers for different resource types.
pub struct Operator<S, E>
where
    S: Clone,
{
    context: Arc<Context<S>>,
    handlers: Vec<Box<dyn DynEventHandler<E>>>,
}

impl<S, E> Operator<S, E>
where
    S: Clone + Send + Sync + 'static,
    E: StdError + Send + Sync + 'static,
{
    /// Creates a new operator
    pub fn new(context: Arc<Context<S>>) -> Self {
        Self {
            context,
            handlers: Default::default(),
        }
    }

    /// Registers an event handler for a resource type
    ///
    /// Chain multiple calls to register handlers for different resources.
    #[must_use]
    pub fn handler<H, K>(mut self, _: H) -> Self
    where
        H: EventHandler<K, S, E> + Send + Sync + 'static,
        K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
        K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    {
        let wrapper = EventHandlerWrapper::<H, K, S, E>::new(self.context.clone());
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

#[doc(hidden)]
pub struct OperatorBuilder;

impl OperatorBuilder {
    /// Attach context to operator builder
    pub fn with_context<S>(self, context: impl Into<Context<S>>) -> OperatorBuilderWithContext<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let context = Arc::new(context.into());
        OperatorBuilderWithContext { context }
    }
}

impl Operator<(), ()> {
    /// Start building an Operator
    #[must_use]
    pub const fn builder() -> OperatorBuilder {
        OperatorBuilder
    }
}

#[doc(hidden)]
pub struct OperatorBuilderWithContext<S>
where
    S: Clone + Send + Sync + 'static,
{
    context: Arc<Context<S>>,
}

impl<S> OperatorBuilderWithContext<S>
where
    S: Clone + Send + Sync + 'static,
{
    /// Registers an event handler for a resource type
    ///
    /// Chain multiple calls to register handlers for different resources.
    #[must_use]
    pub fn handler<H, K, E>(self, _: H) -> Operator<S, E>
    where
        H: EventHandler<K, S, E> + Send + Sync + 'static,
        E: StdError + Send + Sync + 'static,
        K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
        K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    {
        let wrapper = EventHandlerWrapper::<H, K, S, E>::new(self.context.clone());
        let mut operator = Operator::<S, E>::new(self.context);
        operator.handlers.push(Box::new(wrapper));
        operator
    }
}

async fn patch_object<K>(api: &Api<K>, obj: Arc<K>, patch: serde_json::Value) -> kube::Result<()>
where
    K: Resource + Clone + Debug + Serialize + DeserializeOwned,
{
    api.patch::<K>(
        &obj.meta().name.clone().unwrap(),
        &PatchParams::default(),
        &Patch::Json(serde_json::from_value(patch).unwrap()),
    )
    .await?;

    Ok(())
}

pub async fn apply_finalizer<K>(api: &Api<K>, name: &str, obj: Arc<K>) -> kube::Result<()>
where
    K: Resource + Clone + Debug + Serialize + DeserializeOwned,
{
    let patch = if obj.finalizers().is_empty() {
        json!([
            { "op": "test", "path": "/metadata/finalizers", "value": null },
            { "op": "add", "path": "/metadata/finalizers", "value": [name] }
        ])
    } else {
        json!([
            { "op": "test", "path": "/metadata/finalizers", "value": obj.finalizers() },
            { "op": "add", "path": "/metadata/finalizers", "value": name }
        ])
    };

    patch_object(api, obj, patch).await
}

pub async fn remove_finalizer<K>(api: &Api<K>, name: &str, obj: Arc<K>) -> kube::Result<()>
where
    K: Resource + Clone + Debug + Serialize + DeserializeOwned,
{
    let Some(idx) = obj
        .finalizers()
        .iter()
        .enumerate()
        .find(|(_, n)| n == &name)
        .map(|(idx, _)| idx)
    else {
        return Ok(());
    };

    let finalizer_path = format!("/metadata/finalizers/{idx}");

    let patch = json!([
      { "op": "test", "path": finalizer_path, "value": name },
      { "op": "remove", "path": finalizer_path }
    ]);

    patch_object(api, obj, patch).await
}
