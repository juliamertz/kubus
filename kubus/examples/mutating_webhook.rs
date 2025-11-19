//! Mutating admission webhook example
//!
//! This example demonstrates a mutating admission webhook that automatically
//! injects labels into Pod resources before they are created.

use kube::Client;
use kubus::{HandlerError, Operator};

#[tokio::main]
async fn main() -> Result<(), kubus::Error> {
    tracing_subscriber::fmt().init();

    let client = Client::try_default().await?;

    Operator::builder()
        .with_context(client)
        .mutator(inject_label)
        .run()
        .await
}

/// Mutating admission webhook: injects a label into all pods
#[kubus::admission(mutating)]
async fn inject_label(
    req: &kube::core::admission::AdmissionRequest<kube::api::DynamicObject>,
) -> Result<kube::core::admission::AdmissionResponse, HandlerError> {
    use json_patch::{Patch, PatchOperation, AddOperation};
    use json_patch::jsonptr::PointerBuf;
    use serde_json::json;

    // Check if the label already exists
    if let Some(obj) = &req.object {
        if let Some(labels) = obj
            .data
            .get("metadata")
            .and_then(|m| m.get("labels"))
            .and_then(|l| l.as_object())
        {
            if labels.contains_key("kubus.io/injected") {
                // Label already exists, no patch needed
                return Ok(kube::core::admission::AdmissionResponse::from(req));
            }
        }
    }

    // Add the label (use ~1 for / in JSON patch paths)
    let patches = Patch(vec![PatchOperation::Add(AddOperation {
        path: PointerBuf::parse("/metadata/labels/kubus.io~1injected").unwrap(),
        value: json!("true"),
    })]);

    kube::core::admission::AdmissionResponse::from(req)
        .with_patch(patches)
        .map_err(|e| HandlerError::KubusError(kubus::Error::SerializationError(
            serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to serialize JSON patch: {}", e),
            ))
        )))
}
