use std::error::Error as StdError;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::task::JoinSet;
use tracing::Instrument;
use warp::Filter;

#[cfg(feature = "admission")]
use crate::admission::{MutatingAdmissionHandler, ValidatingAdmissionHandler};
use crate::context::Context;
use crate::event_handler::Runnable;

/// Kubernetes operator that manages multiple handlers
pub struct Operator<S, E>
where
    S: Clone,
{
    context: Arc<Context<S>>,
    handlers: Vec<Box<dyn Runnable<S, E>>>,
    #[cfg(feature = "admission")]
    mutating_handlers: Vec<Box<dyn MutatingAdmissionHandler<Err = E>>>,
    #[cfg(feature = "admission")]
    validating_handlers: Vec<Box<dyn ValidatingAdmissionHandler<Err = E>>>,
    #[cfg(feature = "admission")]
    tls_certs_path: Option<PathBuf>,
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
            #[cfg(feature = "admission")]
            mutating_handlers: Vec::new(),
            #[cfg(feature = "admission")]
            validating_handlers: Vec::new(),
            #[cfg(feature = "admission")]
            tls_certs_path: None,
        }
    }

    /// Registers an event handler
    #[must_use]
    pub fn handler<H>(mut self, handler: H) -> Self
    where
        H: Runnable<S, E> + Send + Sync + 'static,
    {
        self.handlers.push(Box::new(handler));
        self
    }

    /// Registers a mutating admission webhook handler
    #[cfg(feature = "admission")]
    #[must_use]
    pub fn mutator<H>(mut self, handler: H) -> Self
    where
        H: MutatingAdmissionHandler<Err = E> + 'static,
    {
        self.mutating_handlers.push(Box::new(handler));
        self
    }

    /// Registers a validating admission webhook handler
    #[cfg(feature = "admission")]
    #[must_use]
    pub fn validator<H>(mut self, handler: H) -> Self
    where
        H: ValidatingAdmissionHandler<Err = E> + 'static,
    {
        self.validating_handlers.push(Box::new(handler));
        self
    }

    /// Sets the path to TLS certificates for the admission webhook server
    ///
    /// The directory should contain `tls.crt` and `tls.key` files.
    /// If not set, the server will run without TLS (useful for local development).
    #[cfg(feature = "admission")]
    #[must_use]
    pub fn with_tls_certs(mut self, path: impl Into<PathBuf>) -> Self {
        self.tls_certs_path = Some(path.into());
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

        #[cfg(feature = "admission")]
        if !self.mutating_handlers.is_empty() || !self.validating_handlers.is_empty() {
            tracing::info!(
                mutating = self.mutating_handlers.len(),
                validating = self.validating_handlers.len(),
                "starting admission webhook server"
            );

            let validate_handler =
                crate::admission::create_validating_route(self.validating_handlers);
            let mutate_handler = crate::admission::create_mutating_route(self.mutating_handlers);

            let addr = ([0, 0, 0, 0], 8443);
            let routes = warp::post()
                .and(
                    warp::path("mutate")
                        .and(warp::body::json())
                        .and_then(mutate_handler)
                        .with(warp::trace::request()),
                )
                .or(warp::path("validate")
                    .and(warp::body::json())
                    .and_then(validate_handler)
                    .with(warp::trace::request()));

            if let Some(path) = self.tls_certs_path {
                set.spawn(
                    warp::serve(routes)
                        .tls()
                        .cert_path(path.join("tls.crt"))
                        .key_path(path.join("tls.key"))
                        .run(addr),
                );
            } else {
                tracing::warn!(
                    "admission webhook server running without TLS - not suitable for production"
                );
                set.spawn(warp::serve(routes).run(addr));
            }
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
    /// Registers an event handler
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

    /// Registers a mutating admission webhook handler
    #[cfg(feature = "admission")]
    #[must_use]
    pub fn mutator<H, E>(self, handler: H) -> Operator<S, E>
    where
        H: MutatingAdmissionHandler<Err = E> + 'static,
        E: StdError + Send + Sync + 'static,
    {
        let mut operator = Operator::<S, E>::new(self.context);
        operator.mutating_handlers.push(Box::new(handler));
        operator
    }

    /// Registers a validating admission webhook handler
    #[cfg(feature = "admission")]
    #[must_use]
    pub fn validator<H, E>(self, handler: H) -> Operator<S, E>
    where
        H: ValidatingAdmissionHandler<Err = E> + 'static,
        E: StdError + Send + Sync + 'static,
    {
        let mut operator = Operator::<S, E>::new(self.context);
        operator.validating_handlers.push(Box::new(handler));
        operator
    }

    /// Sets the path to TLS certificates for the admission webhook server
    #[cfg(feature = "admission")]
    #[must_use]
    pub fn with_tls_certs<H, E>(
        self,
        path: impl Into<PathBuf>,
    ) -> OperatorBuilderWithContextAndTls<S>
    where
        E: StdError + Send + Sync + 'static,
    {
        OperatorBuilderWithContextAndTls {
            context: self.context,
            tls_certs_path: Some(path.into()),
        }
    }
}

#[doc(hidden)]
#[cfg(feature = "admission")]
pub struct OperatorBuilderWithContextAndTls<S>
where
    S: Clone + Send + Sync + 'static,
{
    context: Arc<Context<S>>,
    tls_certs_path: Option<PathBuf>,
}

#[cfg(feature = "admission")]
impl<S> OperatorBuilderWithContextAndTls<S>
where
    S: Clone + Send + Sync + 'static,
{
    /// Registers an event handler
    #[must_use]
    pub fn handler<H, E>(self, handler: H) -> Operator<S, E>
    where
        H: Runnable<S, E> + Send + Sync + 'static,
        E: StdError + Send + Sync + 'static,
    {
        let mut operator = Operator::<S, E>::new(self.context);
        operator.handlers.push(Box::new(handler));
        operator.tls_certs_path = self.tls_certs_path;
        operator
    }
}
