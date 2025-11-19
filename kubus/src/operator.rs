use std::error::Error as StdError;
use std::fmt::Debug;
use std::hash::Hash;
#[cfg(feature = "admission")]
use std::path::PathBuf;
use std::sync::Arc;

use kube::Resource;
use serde::de::DeserializeOwned;
use tokio::task::JoinSet;
use tracing::Instrument;
use warp::Filter;

use crate::Named;
#[cfg(feature = "admission")]
use crate::admission::{create_mutate_handler, AdmissionHandler, AdmissionHandlerWrapper, DynAdmissionHandler};
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
    event_handlers: Vec<Box<dyn DynEventHandler<E>>>,
    #[cfg(feature = "admission")]
    mutating_admission_handlers: Vec<Box<dyn DynAdmissionHandler<E>>>,
    #[cfg(feature = "admission")]
    tls_certs_path: Option<PathBuf>,
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
            event_handlers: Default::default(),
            #[cfg(feature = "admission")]
            mutating_admission_handlers: Default::default(),
            #[cfg(feature = "admission")]
            tls_certs_path: Default::default(),
        }
    }

    /// Registers an event handler for a resource type
    ///
    /// Chain multiple calls to register handlers for different resources.
    #[must_use]
    pub fn handler<H, K>(mut self, _: H) -> Self
    where
        H: EventHandler<K, S, E> + Named + Send + Sync + 'static,
        K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
        K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    {
        let wrapper = EventHandlerWrapper::<H, K, S, E>::new(self.context.clone());
        self.event_handlers.push(Box::new(wrapper));
        self
    }

    /// Configure path to TLS certificates to be used for the webhook server
    #[cfg(feature = "admission")]
    pub fn with_tls(mut self, path: PathBuf) -> Self {
        self.tls_certs_path = Some(path);
        self
    }

    /// Configure path to TLS certificates to be used for the webhook server if `path` is `Some`
    #[cfg(feature = "admission")]
    pub fn with_optional_tls(mut self, path: Option<PathBuf>) -> Self {
        self.tls_certs_path = path;
        self
    }

    #[cfg(feature = "admission")]
    pub fn mutator<H>(mut self, _: H) -> Self
    where
        E: StdError + Sync + Send + 'static,
        H: AdmissionHandler<E> + Named + Sync + Send + 'static,
    {
        let wrapper = AdmissionHandlerWrapper::<E, H>::default();
        self.mutating_admission_handlers.push(Box::new(wrapper));
        self
    }
    // pub fn mutator<H>(mut self, handler: H) -> Self
    // where
    //     H: AdmissionHandler + Send + Sync + 'static,
    // {
    //     self.mutating_admission_handlers.push(Box::new(handler));
    //     self
    // }

    /// Runs the operator, starting all registered handlers
    ///
    /// Blocks until all handlers complete (typically on shutdown).
    pub async fn run(self) -> crate::Result<()> {
        tracing::info!(
            "starting kubus operator with {} handlers",
            self.event_handlers.len()
        );

        let mut set = JoinSet::new();

        for handler in self.event_handlers {
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

        #[cfg(feature = "admission")]
        if !self.mutating_admission_handlers.is_empty() {
            tracing::info!("starting admission server");

            let mutate_handler = create_mutate_handler(self.mutating_admission_handlers);

            let addr = ([0, 0, 0, 0], 8443);
            let routes = warp::post().and(
                warp::path("mutate")
                    .and(warp::body::json())
                    .and_then(mutate_handler)
                    .with(warp::trace::request()),
            );

            if let Some(path) = self.tls_certs_path {
                set.spawn(
                    warp::serve(routes)
                        .tls()
                        .cert_path(path.join("tls.crt"))
                        .key_path(path.join("tls.key"))
                        .run(addr),
                );
            } else {
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
        H: EventHandler<K, S, E> + Named + Send + Sync + 'static,
        E: StdError + Send + Sync + 'static,
        K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
        K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
    {
        let wrapper = EventHandlerWrapper::<H, K, S, E>::new(self.context.clone());
        let mut operator = Operator::<S, E>::new(self.context);
        operator.event_handlers.push(Box::new(wrapper));
        operator
    }
}
