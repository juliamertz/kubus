use std::sync::Arc;

use k8s_openapi::api::core::v1::Pod;
use kube::{Client, ResourceExt, api::DynamicObject, core::admission::AdmissionResponse};
use kubus::{admission::AdmissionHandler, kubus, webhook, Context, HandlerError, Named, Operator};

#[derive(Debug, Clone)]
struct State {}

#[tokio::main]
async fn main() -> Result<(), kubus::Error> {
    tracing_subscriber::fmt().init();

    let client = Client::try_default().await?;
    let state = State {};

    Operator::builder()
        .with_context((client, state))
        .handler(on_pod_apply)
        .handler(on_pod_delete)
        .mutator(MyHandler)
        .run()
        .await
}

struct MyHandler;

impl Named for MyHandler {
    const NAME: &str = "MyHandler";
}

#[async_trait::async_trait]
impl AdmissionHandler for MyHandler {
    type Err = HandlerError;

    async fn handle(
        &self,
        res: AdmissionResponse,
        obj: &DynamicObject,
    ) -> Result<AdmissionResponse, HandlerError> {
        dbg!(&res, obj);
        Ok(res)
    }
}

#[webhook(mutating)]
fn derive_handler() {}

#[kubus(
    event = Apply,
    label_selector = "app.kubernetes.io/managed-by=kubus",
    finalizer = "kubus.io/cleanup",
)]
async fn on_pod_apply(pod: Arc<Pod>, _ctx: Arc<Context<State>>) -> Result<(), HandlerError> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("apply event for pod {name} in {namespace}");

    Ok(())
}

#[kubus(
    event = Delete,
    label_selector = "app.kubernetes.io/managed-by=kubus",
    finalizer = "kubus.io/cleanup"
)]
async fn on_pod_delete(pod: Arc<Pod>, _ctx: Arc<Context<State>>) -> Result<(), HandlerError> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("delete event for pod {name} in {namespace}");

    Ok(())
}
