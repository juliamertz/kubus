use std::{sync::Arc, time::Duration};

use k8s_openapi::api::core::v1::Pod;
use kube::{Client, ResourceExt, runtime::controller::Action};
use kubus::{Context, HandlerError, Operator, kubus};

#[derive(Debug, Clone)]
struct State {}

#[tokio::main]
async fn main() -> Result<(), kubus::Error> {
    let client = Client::try_default().await?;
    let state = State {};

    Operator::builder()
        .with_context((client, state))
        .handler(on_pod_apply)
        .handler(on_pod_delete)
        .run()
        .await
}

#[kubus(
    event = Apply,
    label_selector = "app.kubernetes.io/managed-by=kubus",
    finalizer = "kubus.io/cleanup",
)]
async fn on_pod_apply(pod: Arc<Pod>, _ctx: Arc<Context<State>>) -> Result<Action, HandlerError> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("apply event for pod {name} in {namespace}");

    Ok(Action::requeue(Duration::from_secs(5)))
}

#[kubus(
    event = Delete,
    label_selector = "app.kubernetes.io/managed-by=kubus",
    finalizer = "kubus.io/cleanup"
)]
async fn on_pod_delete(pod: Arc<Pod>, _ctx: Arc<Context<State>>) -> Result<Action, HandlerError> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("delete event for pod {name} in {namespace}");

    Ok(Action::requeue(Duration::from_secs(5)))
}
