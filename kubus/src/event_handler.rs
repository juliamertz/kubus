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

use crate::{Context, Error};

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

/// Trait for handling Kubernetes resource events
///
/// Implemented automatically by the `#[kubus]` macro.
#[async_trait]
pub trait Handler<K, S, E = HandlerError>
where
    K: Resource + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
    K::DynamicType: Default + Debug + Eq + Hash + Clone + Unpin,
    S: Clone + Send + Sync + 'static,
    E: StdError + Send + Sync + 'static,
{
    const NAME: &'static str;
    const LABEL_SELECTOR: Option<&'static str> = None;
    const FIELD_SELECTOR: Option<&'static str> = None;

    /// Handles a resource event
    async fn handle(resource: Arc<K>, context: Arc<Context<S>>) -> Result<Action, E>;

    /// Error handling policy
    fn error_policy(_resource: Arc<K>, err: &E, _ctx: Arc<Context<S>>) -> Action {
        tracing::error!({ err = err as &dyn StdError }, "handler error");
        Action::requeue(Duration::from_secs(5))
    }

    /// Runs the handler with a Kubernetes client
    async fn run(&self, client: Client, context: Arc<Context<S>>) -> Result<(), E>
    where
        Self: Sized + 'static,
    {
        let api = Api::<K>::all(client);
        let mut config = Config::default();
        config.label_selector = Self::LABEL_SELECTOR.map(String::from);
        config.field_selector = Self::FIELD_SELECTOR.map(String::from);

        Controller::new(api, config)
            .shutdown_on_signal()
            .run(Self::handle, Self::error_policy, context)
            .for_each(|_| futures::future::ready(()))
            .await;

        Ok(())
    }
}

/// Type-erased trait for running handlers
///
/// Automatically implemented for types that implement `Handler<K, S, E>`.
#[async_trait]
pub trait Runnable<S, E>: Send + Sync
where
    S: Clone + Send + Sync + 'static,
    E: StdError + Send + Sync + 'static,
{
    fn name(&self) -> &'static str;
    async fn run(&self, client: Client, context: Arc<Context<S>>) -> Result<(), E>;
}
