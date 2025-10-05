use std::pin::Pin;

use futures::future;
use kube::Client;

pub use inventory;
pub use kube;
pub mod finalizer;

pub use finalizer::update_finalizer;
pub use kubus_derive::kubus;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

inventory::collect!(Handler);

#[derive(Clone)]
pub struct Handler {
    pub name: &'static str,
    pub watch_fn: fn(Client) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>,
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
