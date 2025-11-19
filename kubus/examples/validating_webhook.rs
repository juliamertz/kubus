//! Validating admission webhook example
//!
//! This example demonstrates a validating admission webhook that enforces
//! security policies by rejecting Pods that attempt to run as root.

use kube::Client;
use kubus::{HandlerError, Operator};

#[tokio::main]
async fn main() -> Result<(), kubus::Error> {
    tracing_subscriber::fmt().init();

    let client = Client::try_default().await?;

    Operator::builder()
        .with_context(client)
        .validator(validate_security)
        .run()
        .await
}

/// Validating admission webhook: ensures pods don't run as root
#[kubus::admission(validating)]
async fn validate_security(
    req: &kube::core::admission::AdmissionRequest<kube::api::DynamicObject>,
) -> Result<kube::core::admission::AdmissionResponse, HandlerError> {
    // Only validate Pod resources
    if let Some(obj) = &req.object {
        let kind = obj.types.as_ref().map(|t| t.kind.as_str()).unwrap_or("");
        if kind != "Pod" {
            return Ok(kube::core::admission::AdmissionResponse::from(req));
        }

        // Check if any container runs as root (runAsUser = 0)
        if let Some(spec) = obj.data.get("spec") {
            if let Some(security_context) = spec.get("securityContext") {
                if let Some(run_as_user) = security_context.get("runAsUser") {
                    if run_as_user.as_u64() == Some(0) {
                        return Ok(
                            kube::core::admission::AdmissionResponse::from(req)
                                .deny("Pods cannot run as root (runAsUser=0)")
                        );
                    }
                }
            }

            // Check containers
            if let Some(containers) = spec.get("containers").and_then(|c| c.as_array()) {
                for container in containers {
                    if let Some(security_context) = container.get("securityContext") {
                        if let Some(run_as_user) = security_context.get("runAsUser") {
                            if run_as_user.as_u64() == Some(0) {
                                return Ok(
                                    kube::core::admission::AdmissionResponse::from(req)
                                        .deny("Containers cannot run as root (runAsUser=0)")
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(kube::core::admission::AdmissionResponse::from(req))
}
