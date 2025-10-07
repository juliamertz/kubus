use std::{error::Error, sync::Arc, time::Duration};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use kube::{Client, runtime::controller::Action};
use kubus::{EventHandler, Operator, Result};

struct Context {
    _do_thing: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let context = Context { _do_thing: true };
    let client = Client::try_default().await?;
    let operator = Operator::new(client, context).handler(PodApplyHandler);

    operator.run().await?;

    Ok(())
}

struct PodApplyHandler;

#[async_trait]
impl EventHandler<Pod, Context> for PodApplyHandler {
    async fn handler(pod: Arc<Pod>, _context: Arc<Context>) -> Result<Action> {
        dbg!(pod, "Pod apply event received");
        Ok(Action::requeue(Duration::from_secs(5)))
    }
}
