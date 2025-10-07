use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

use futures::future;
use kube::Resource;
use serde::de::DeserializeOwned;

use crate::event_handler::{DynEventHandler, EventHandler, EventHandlerWrapper};
use crate::{Context, Result};

/// Kubernetes operator managing multiple resource handlers
///
/// Use with the `#[kubus]` derive macro to register handlers for different resource types.
pub struct Operator<Ctx, Err>
where
    Ctx: Clone,
{
    context: Arc<Context<Ctx>>,
    handlers: Vec<Box<dyn DynEventHandler<Err>>>,
}

impl<Ctx, Err> Operator<Ctx, Err>
where
    Ctx: Clone + Send + Sync + 'static,
    Err: std::error::Error + Send + Sync + 'static,
{
    /// Creates a new operator
    pub fn new(context: Arc<Context<Ctx>>) -> Self {
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

pub struct OperatorBuilder;

impl OperatorBuilder {
    /// Attach context to operator builder
    pub fn with_context<Ctx>(
        self,
        context: impl Into<Context<Ctx>>,
    ) -> OperatorBuilderWithContext<Ctx>
    where
        Ctx: Clone + Send + Sync + 'static,
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

pub struct OperatorBuilderWithContext<Ctx>
where
    Ctx: Clone + Send + Sync + 'static,
{
    context: Arc<Context<Ctx>>,
}

impl<Ctx> OperatorBuilderWithContext<Ctx>
where
    Ctx: Clone + Send + Sync + 'static,
{
    /// Registers an event handler for a resource type
    ///
    /// Chain multiple calls to register handlers for different resources.
    #[must_use]
    pub fn handler<H, K, Err>(self, _: H) -> Operator<Ctx, Err>
    where
        H: EventHandler<K, Ctx, Err> + Send + Sync + 'static,
        Err: std::error::Error + Send + Sync + 'static,
        K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
        K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    {
        let wrapper = EventHandlerWrapper::<H, K, Ctx, Err>::new(self.context.clone());
        let mut operator = Operator::<Ctx, Err>::new(self.context);
        operator.handlers.push(Box::new(wrapper));
        operator
    }
}
