use std::{sync::Arc, time::Duration};

use k8s_openapi::api::core::v1::Pod;
use kube::{Client, ResourceExt, runtime::controller::Action};
use kubus::{kubus, Context, Operator, Result};

struct State {
    _things: Vec<i32>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::try_default().await?;
    let state = State { _things: vec![] };

    Operator::new(client, state)
        .handler(apply_pod)
        .handler(cleanup_pod)
        .run()
        .await?;

    Ok(())
}

#[kubus(event = Apply, finalizer = "kubus.io/cleanup")]
async fn apply_pod(pod: Arc<Pod>, _ctx: Arc<Context<State>>) -> Result<Action> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("apply event for pod {name} in {namespace}");

    Ok(Action::requeue(Duration::from_secs(5)))
}

#[kubus(event = Delete, finalizer = "kubus.io/cleanup")]
async fn cleanup_pod(pod: Arc<Pod>, _ctx: Arc<Context<State>>) -> Result<Action> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("delete event for pod {name} in {namespace}");

    Ok(Action::requeue(Duration::from_secs(5)))
}
