use std::error::Error as StdError;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

use kube::Resource;
use serde::de::DeserializeOwned;
use tokio::task::JoinSet;
use tracing::Instrument;

use crate::context::Context;
use crate::event_handler::{DynEventHandler, EventHandler, EventHandlerWrapper};

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
    pub async fn run(self) -> crate::Result<()> {
        tracing::info!(
            "starting kubus operator with {} handlers",
            self.handlers.len()
        );

        let mut set = JoinSet::new();

        for handler in self.handlers {
            let client = self.context.client.clone();
            let span = tracing::info_span!("handler", name = handler.name());
            let task = async move {
                let client = client.clone();
                if let Err(err) = handler.run(client).await {
                    tracing::error!({ err = &err as &dyn StdError }, "handler error");
                }
            };

            set.spawn(task.instrument(span));
        }

        set.join_all().await;

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
