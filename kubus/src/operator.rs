use std::error::Error as StdError;
use std::sync::Arc;

use tokio::task::JoinSet;
use tracing::Instrument;

use crate::context::Context;
use crate::event_handler::Runnable;

/// Kubernetes operator that manages multiple handlers
pub struct Operator<S, E>
where
    S: Clone,
{
    context: Arc<Context<S>>,
    handlers: Vec<Box<dyn Runnable<S, E>>>,
}

impl<S, E> Operator<S, E>
where
    S: Clone + Send + Sync + 'static,
    E: StdError + Send + Sync + 'static,
{
    pub fn new(context: Arc<Context<S>>) -> Self {
        Self {
            context,
            handlers: Vec::new(),
        }
    }

    /// Registers a handler
    #[must_use]
    pub fn handler<H>(mut self, handler: H) -> Self
    where
        H: Runnable<S, E> + Send + Sync + 'static,
    {
        self.handlers.push(Box::new(handler));
        self
    }

    /// Runs the operator, starting all registered handlers
    pub async fn run(self) -> crate::Result<()> {
        tracing::info!("starting operator with {} handlers", self.handlers.len());

        let mut set = JoinSet::new();

        for handler in self.handlers {
            let client = self.context.client.clone();
            let context = self.context.clone();
            let name = handler.name();
            let span = tracing::info_span!("handler", name);

            set.spawn(
                async move {
                    if let Err(err) = handler.run(client, context).await {
                        tracing::error!({ err = &err as &dyn StdError }, "handler error");
                    }
                }
                .instrument(span),
            );
        }

        set.join_all().await;
        Ok(())
    }
}

#[doc(hidden)]
pub struct OperatorBuilder;

impl OperatorBuilder {
    pub fn with_context<S>(self, context: impl Into<Context<S>>) -> OperatorBuilderWithContext<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let context = Arc::new(context.into());
        OperatorBuilderWithContext { context }
    }
}

impl Operator<(), ()> {
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
    /// Registers a handler
    #[must_use]
    pub fn handler<H, E>(self, handler: H) -> Operator<S, E>
    where
        H: Runnable<S, E> + Send + Sync + 'static,
        E: StdError + Send + Sync + 'static,
    {
        let mut operator = Operator::<S, E>::new(self.context);
        operator.handlers.push(Box::new(handler));
        operator
    }
}
