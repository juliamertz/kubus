use std::{sync::Arc, time::Duration};

use k8s_openapi::api::core::v1::Pod;
use kube::{ResourceExt, runtime::controller::Action};
use kubus::{Context, Result, kubus};

kubus::main!();

#[kubus(event = Apply, finalizer = "kubus.io/cleanup")]
async fn on_pod_apply(pod: Arc<Pod>, ctx: Arc<Context>) -> Result<Action> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("pod {name} has been created in {namespace}");

    Ok(Action::requeue(Duration::from_secs(300)))
}

#[kubus(event = Delete, finalizer = "kubus.io/cleanup")]
async fn on_pod_delete(pod: Arc<Pod>, ctx: Arc<Context>) -> Result<Action> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    tracing::info!("pod {name} is being deleted from {namespace}");

    Ok(Action::requeue(Duration::from_secs(300)))
}
