use std::error::Error as StdError;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher::Config;
use kube::{Api, Client, Resource};
use serde::de::DeserializeOwned;

use crate::{Context, Error, Named};

/// Errors that can occur during event handler execution
#[derive(thiserror::Error, Debug)]
pub enum HandlerError {
    /// Error from the Kubernetes client
    #[error("Kube Error: {0}")]
    KubeError(#[from] kube::Error),
    /// Error from the Kubus
    #[error("Kubus Error: {0}")]
    KubusError(#[from] Error),
}

/// Kubernetes resource event types
#[derive(Debug, PartialEq, Eq)]
pub enum EventType {
    /// Resource created or updated
    Apply,
    /// Resource deleted
    Delete,
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

pub(crate) struct EventHandlerWrapper<H, K, S, E>
where
    H: EventHandler<K, S, E> + Named,
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
    H: EventHandler<K, S, E> + Named,
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    S: Clone + Send + Sync + 'static,
    E: StdError + Sync + Send + 'static,
{
    pub(crate) const fn new(context: Arc<Context<S>>) -> Self {
        Self {
            context,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
pub(crate) trait DynEventHandler<E>: Send + Sync
where
    E: StdError + Send + Sync + 'static,
{
    fn name(&self) -> &'static str;

    async fn run(&self, client: Client) -> Result<(), E>;
}

#[async_trait]
impl<H, K, S, E> DynEventHandler<E> for EventHandlerWrapper<H, K, S, E>
where
    H: EventHandler<K, S, E> + Named + Send + Sync + 'static,
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    S: Clone + Send + Sync + 'static,
    E: StdError + Sync + Send + 'static,
{
    fn name(&self) -> &'static str {
        H::NAME
    }

    async fn run(&self, client: Client) -> Result<(), E> {
        let context = self.context.clone();
        H::watch(client, context).await
    }
}
