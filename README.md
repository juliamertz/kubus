# Kubus

A small Rust library for building Kubernetes operators with less boilerplate.

## Example

```rust
use std::{sync::Arc};

use k8s_openapi::api::core::v1::Pod;
use kube::{Client, ResourceExt, runtime::controller::Action};
use kubus::{Context, HandlerError, Operator, kubus};

struct State {}

#[tokio::main]
async fn main() -> Result<(), kubus::Error> {
    let client = Client::try_default().await?;
    let state = State {};

    Operator::builder()
        .with_context((client, state))
        .handler(on_pod_apply)
        .run()
        .await
}

#[kubus(event = Apply, finalizer = "kubus.io/cleanup")]
async fn on_pod_apply(pod: Arc<Pod>, _ctx: Arc<Context<State>>) -> Result<(), HandlerError> {
    let name = pod.name_unchecked();
    let namespace = pod.namespace().unwrap();

    println!("apply event for pod {name} in {namespace}");

    Ok(())
}
```

The `#[kubus]` attribute handles the watcher setup, reconciliation loop, and finalizer management. You just write functions that handle events.

## Status

Experimental - the API will probably change as I figure out what works.

## Thanks to

- [kopf](https://github.com/nolar/kopf) For the inspiration
- [kube-rs](https://github.com/kube-rs/kube) For the amazing library
