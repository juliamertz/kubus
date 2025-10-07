use std::{fmt::Debug, hash::Hash, sync::Arc, time::Duration};

use crate::Context;

use async_trait::async_trait;
use futures::StreamExt;
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher::Config as ControllerConfig;
use kube::{Api, Client, Resource};
use serde::de::DeserializeOwned;

/// Kubernetes resource event types
#[derive(Debug, PartialEq, Eq)]
pub enum EventType {
    /// Resource created or updated
    Apply,
    /// Resource deleted
    Delete,
}

#[async_trait]
pub(crate) trait DynEventHandler<Err>: Send + Sync
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
pub trait EventHandler<K, Ctx, Err = crate::HandlerError>
where
    K: Resource + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    Ctx: Clone + Send + Sync + 'static,
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

pub(crate) struct EventHandlerWrapper<H, K, Ctx, Err>
where
    H: EventHandler<K, Ctx, Err>,
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    Ctx: Clone + Send + Sync + 'static,
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
    Ctx: Clone + Send + Sync + 'static,
    Err: std::error::Error + Sync + Send + 'static,
{
    pub(crate) const fn new(context: Arc<Context<Ctx>>) -> Self {
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
    Ctx: Clone + Send + Sync + 'static,
    Err: std::error::Error + Sync + Send + 'static,
{
    async fn run(&self, client: Client) -> Result<(), Err> {
        let context = self.context.clone();
        H::watch(client, context).await
    }
}
