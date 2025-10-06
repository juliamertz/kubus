# Kubus

A small Rust library for building Kubernetes operators with less boilerplate.

## Example

```rust
use k8s_openapi::api::core::v1::Pod;
use kube::{Client, ResourceExt};
use kubus::{Result, kubus};

kubus::main!();

#[kubus(event = Apply, finalizer = "kubus.io/cleanup")]
fn on_pod_apply(_client: Client, pod: Pod) -> Result<()> {
    let name = pod.name_unchecked();
    let namespace = pod
        .namespace()
        .to_owned()
        .unwrap_or_else(|| "default".into());

    println!("pod {name} has been created in {namespace}");
    Ok(())
}
```

The `#[kubus]` attribute handles the watcher setup, reconciliation loop, and finalizer management. You just write functions that handle events.

## Status

Experimental - the API will probably change as I figure out what works.
