use std::{sync::Arc, time::Duration};

use k8s_openapi::api::core::v1::Pod;
use kube::{Client, ResourceExt, runtime::controller::Action};
use kubus::{Context, HandlerError, Operator, kubus};

struct State {}

#[tokio::main]
async fn main() -> Result<(), kubus::Error> {
    let client = Client::try_default().await?;
    let state = State {};

    Operator::new(client, state)
        .handler(apply_pod)
        .handler(cleanup_pod)
        .run()
        .await
}

#[kubus(event = Apply, finalizer = "kubus.io/cleanup")]
async fn apply_pod(pod: Arc<Pod>, _ctx: Arc<Context<State>>) -> Result<Action, HandlerError> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("apply event for pod {name} in {namespace}");

    Ok(Action::requeue(Duration::from_secs(5)))
}

#[kubus(event = Delete, finalizer = "kubus.io/cleanup")]
async fn cleanup_pod(pod: Arc<Pod>, _ctx: Arc<Context<State>>) -> Result<Action, HandlerError> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("delete event for pod {name} in {namespace}");

    Ok(Action::requeue(Duration::from_secs(5)))
}
