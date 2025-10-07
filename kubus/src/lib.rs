use std::{pin::Pin, str::FromStr};

use futures::future;
use kube::Client;

pub use inventory;
pub use kube;
pub mod finalizer;

pub use finalizer::update_finalizer;
pub use kubus_derive::kubus;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("SerializationError: {0}")]
    SerializationError(#[source] serde_json::Error),

    #[error("Kube Error: {0}")]
    KubeError(#[from] kube::Error),

    #[error("Finalizer Error: {0}")]
    // NB: awkward type because finalizer::Error embeds the reconciler error (which is this)
    // so boxing this error to break cycles
    FinalizerError(#[source] Box<kube::runtime::finalizer::Error<Error>>),

    #[error("IllegalDocument")]
    IllegalDocument,
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

inventory::collect!(Handler);

#[derive(Clone)]
pub struct Handler {
    pub name: &'static str,
    pub watch_fn: fn(Client) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>,
}

#[derive(Clone)]
pub struct Context {
    pub client: Client,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EventType {
    Apply,
    InitApply,
    Delete,
}

impl FromStr for EventType {
    type Err = std::io::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "Apply" => Ok(Self::Apply),
            "InitApply" => Ok(Self::InitApply),
            "Delete" => Ok(Self::Delete),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no such event type",
            )),
        }
    }
}

pub struct Operator {
    client: Client,
    handlers: Vec<Handler>,
}

impl Operator {
    pub async fn new() -> Result<Self> {
        let client = Client::try_default().await?;
        Ok(Self {
            client,
            handlers: inventory::iter::<Handler>.into_iter().cloned().collect(),
        })
    }

    pub fn with_client(client: Client) -> Self {
        Self {
            client,
            handlers: inventory::iter::<Handler>.into_iter().cloned().collect(),
        }
    }

    pub async fn run(self) -> Result<()> {
        tracing::info!(
            "starting kubus operator with {} handlers",
            self.handlers.len()
        );

        let tasks: Vec<_> = self
            .handlers
            .into_iter()
            .map(|handler| {
                let client = self.client.clone();
                tokio::spawn(async move {
                    tracing::info!("starting handler: {}", handler.name);
                    loop {
                        let client = client.clone();
                        if let Err(e) = (handler.watch_fn)(client).await {
                            tracing::error!("restarting handler {} failed: {}", handler.name, e);
                        }
                    }
                })
            })
            .collect();

        future::join_all(tasks).await;
        Ok(())
    }
}

#[macro_export]
macro_rules! main {
    () => {
        #[tokio::main]
        async fn main() -> Result<()> {
            tracing_subscriber::fmt::init();
            kubus::Operator::new().await?.run().await
        }
    };
}
