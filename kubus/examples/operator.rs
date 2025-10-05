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
