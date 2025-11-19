use std::convert::Infallible;
use std::error::Error as StdError;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use kube::api::DynamicObject;
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview};
use kube::ResourceExt;
use tracing::{error, info, warn};

/// Trait for mutating admission webhook handlers
///
/// Mutating admission webhooks can modify resources before they are persisted.
/// Common use cases include:
/// - Injecting sidecar containers
/// - Setting default values
/// - Adding labels or annotations
/// - Modifying resource requests/limits
///
/// # Example
///
/// ```rust,no_run
/// use kube::api::DynamicObject;
/// use kube::core::admission::{AdmissionRequest, AdmissionResponse};
/// use kubus::admission::MutatingAdmissionHandler;
/// use kubus::HandlerError;
///
/// struct MyMutator;
///
/// #[async_trait::async_trait]
/// impl MutatingAdmissionHandler for MyMutator {
///     type Err = HandlerError;
///
///     async fn mutate(
///         &self,
///         req: &AdmissionRequest<DynamicObject>,
///     ) -> Result<AdmissionResponse, Self::Err> {
///         // Modify the object and return patches
///         use json_patch::{Patch, PatchOperation, AddOperation};
///         use json_patch::jsonptr::PointerBuf;
///         use serde_json::json;
///         let patches = Patch(vec![
///             PatchOperation::Add(AddOperation {
///                 path: PointerBuf::parse("/metadata/labels/my-label").unwrap(),
///                 value: json!("my-value"),
///             }),
///         ]);
///         Ok(AdmissionResponse::from(req).with_patch(patches)?)
///     }
/// }
/// ```
#[async_trait]
pub trait MutatingAdmissionHandler: Send + Sync {
    /// Error type returned by the handler
    type Err: StdError + Send + Sync + 'static;

    /// Returns the name of this handler for logging purposes
    fn name(&self) -> &'static str;

    /// Mutates the resource in the admission request
    ///
    /// Returns an `AdmissionResponse` that may include JSON patches to modify the resource.
    /// Use `AdmissionResponse::from(req).with_patch(patches)` to apply patches.
    /// If no patches are needed, return `AdmissionResponse::from(req)`.
    async fn mutate(
        &self,
        req: &AdmissionRequest<DynamicObject>,
    ) -> Result<AdmissionResponse, Self::Err>;
}

/// Trait for validating admission webhook handlers
///
/// Validating admission webhooks can accept or reject API requests.
/// Common use cases include:
/// - Enforcing security policies (e.g., no root containers)
/// - Validating resource schemas
/// - Enforcing resource quotas
/// - Validating image sources
///
/// # Example
///
/// ```rust,no_run
/// use kube::api::DynamicObject;
/// use kube::core::admission::{AdmissionRequest, AdmissionResponse};
/// use kubus::admission::ValidatingAdmissionHandler;
/// use kubus::HandlerError;
///
/// struct MyValidator;
///
/// #[async_trait::async_trait]
/// impl ValidatingAdmissionHandler for MyValidator {
///     type Err = HandlerError;
///
///     async fn validate(
///         &self,
///         req: &AdmissionRequest<DynamicObject>,
///     ) -> Result<AdmissionResponse, Self::Err> {
///         // Validate the object
///         if /* some condition */ false {
///             Ok(AdmissionResponse::from(req).deny("Resource does not meet requirements"))
///         } else {
///             Ok(AdmissionResponse::from(req))
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait ValidatingAdmissionHandler: Send + Sync {
    /// Error type returned by the handler
    type Err: StdError + Send + Sync + 'static;

    /// Returns the name of this handler for logging purposes
    fn name(&self) -> &'static str;

    /// Validates the resource in the admission request
    ///
    /// Returns an `AdmissionResponse` that either accepts or denies the request.
    /// Use `AdmissionResponse::from(req).deny(message)` to reject the request.
    async fn validate(
        &self,
        req: &AdmissionRequest<DynamicObject>,
    ) -> Result<AdmissionResponse, Self::Err>;
}

/// Creates a warp route handler for mutating admission webhooks
pub(crate) fn create_mutating_route<E: StdError + Send + Sync + 'static>(
    handlers: Vec<Box<dyn MutatingAdmissionHandler<Err = E>>>,
) -> impl Fn(
    AdmissionReview<DynamicObject>,
) -> std::pin::Pin<Box<dyn Future<Output = Result<warp::reply::Json, Infallible>> + Send>>
+ Clone {
    let handlers = Arc::new(handlers);
    move |body: AdmissionReview<DynamicObject>| {
        let handlers = handlers.clone();
        Box::pin(async move {
            let req: AdmissionRequest<_> = match body.try_into() {
                Ok(req) => req,
                Err(err) => {
                    error!("invalid admission request: {}", err);
                    return Ok(warp::reply::json(
                        &AdmissionResponse::invalid(err.to_string()).into_review(),
                    ));
                }
            };

            let mut res = AdmissionResponse::from(&req);

            if let Some(obj) = &req.object {
                let name = obj.name_any();
                let kind = obj.types.clone().unwrap_or_default().kind;

                for handler in handlers.iter() {
                    match handler.mutate(&req).await {
                        Ok(handler_res) => {
                            // Merge patches from multiple handlers
                            res = merge_responses(res, handler_res);
                            info!(
                                handler = handler.name(),
                                operation = ?req.operation,
                                kind = %kind,
                                name = %name,
                                "mutated resource"
                            );
                        }
                        Err(err) => {
                            error!(
                                handler = handler.name(),
                                operation = ?req.operation,
                                kind = %kind,
                                name = %name,
                                error = %err,
                                "mutation failed"
                            );
                            res = res.deny(format!("{}: {}", handler.name(), err));
                            break;
                        }
                    }
                }
            }

            Ok(warp::reply::json(&res.into_review()))
        })
    }
}

/// Creates a warp route handler for validating admission webhooks
pub(crate) fn create_validating_route<E: StdError + Send + Sync + 'static>(
    handlers: Vec<Box<dyn ValidatingAdmissionHandler<Err = E>>>,
) -> impl Fn(
    AdmissionReview<DynamicObject>,
) -> std::pin::Pin<Box<dyn Future<Output = Result<warp::reply::Json, Infallible>> + Send>>
+ Clone {
    let handlers = Arc::new(handlers);
    move |body: AdmissionReview<DynamicObject>| {
        let handlers = handlers.clone();
        Box::pin(async move {
            let req: AdmissionRequest<_> = match body.try_into() {
                Ok(req) => req,
                Err(err) => {
                    error!("invalid admission request: {}", err);
                    return Ok(warp::reply::json(
                        &AdmissionResponse::invalid(err.to_string()).into_review(),
                    ));
                }
            };

            let mut res = AdmissionResponse::from(&req);

            if let Some(obj) = &req.object {
                let name = obj.name_any();
                let kind = obj.types.clone().unwrap_or_default().kind;

                for handler in handlers.iter() {
                    match handler.validate(&req).await {
                        Ok(handler_res) => {
                            // If any handler denies, the request is denied
                            if !handler_res.allowed {
                                warn!(
                                    handler = handler.name(),
                                    operation = ?req.operation,
                                    kind = %kind,
                                    name = %name,
                                    "validation denied"
                                );
                                res = handler_res;
                                break;
                            }
                            info!(
                                handler = handler.name(),
                                operation = ?req.operation,
                                kind = %kind,
                                name = %name,
                                "validation passed"
                            );
                        }
                        Err(err) => {
                            error!(
                                handler = handler.name(),
                                operation = ?req.operation,
                                kind = %kind,
                                name = %name,
                                error = %err,
                                "validation error"
                            );
                            res = res.deny(format!("{}: {}", handler.name(), err));
                            break;
                        }
                    }
                }
            }

            Ok(warp::reply::json(&res.into_review()))
        })
    }
}

/// Merges two admission responses, combining their patches
///
/// When multiple mutating handlers provide patches, this function combines them
/// into a single patch array. The patches are applied in order, which means
/// later patches can modify values set by earlier patches.
fn merge_responses(mut base: AdmissionResponse, other: AdmissionResponse) -> AdmissionResponse {
    // If the other response denies, use it
    if !other.allowed {
        return other;
    }

    // Combine patches from both responses
    // Both patches are serialized as JSON arrays of patch operations
    if let (Some(base_patch_bytes), Some(other_patch_bytes)) = (base.patch.as_ref(), other.patch.as_ref()) {
        // Deserialize both patches
        if let (Ok(base_patches), Ok(other_patches)) = (
            serde_json::from_slice::<Vec<serde_json::Value>>(base_patch_bytes),
            serde_json::from_slice::<Vec<serde_json::Value>>(other_patch_bytes),
        ) {
            // Combine the patch operations
            let mut combined = base_patches;
            combined.extend_from_slice(&other_patches);
            
            // Re-serialize the combined patches
            if let Ok(combined_bytes) = serde_json::to_vec(&combined) {
                base.patch = Some(combined_bytes);
            }
        }
    } else if other.patch.is_some() {
        // If only the other has patches, use them
        base.patch = other.patch;
    }

    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HandlerError;
    use kube::core::admission::AdmissionRequest;
    use serde_json::json;

    // Test mutating handler
    struct TestMutator;

    #[async_trait::async_trait]
    impl MutatingAdmissionHandler for TestMutator {
        type Err = HandlerError;

        fn name(&self) -> &'static str {
            "TestMutator"
        }

        async fn mutate(
            &self,
            req: &AdmissionRequest<DynamicObject>,
        ) -> Result<AdmissionResponse, Self::Err> {
            use json_patch::{Patch, PatchOperation, AddOperation};
            use json_patch::jsonptr::PointerBuf;

            if req.object.as_ref().and_then(|o| o.metadata.name.as_deref()) == Some("test-pod") {
                let patches = Patch(vec![PatchOperation::Add(AddOperation {
                    path: PointerBuf::parse("/metadata/labels/test-label").unwrap(),
                    value: json!("test-value"),
                })]);
                Ok(AdmissionResponse::from(req).with_patch(patches).map_err(|e| {
                    HandlerError::KubusError(crate::Error::SerializationError(
                        serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to serialize patch: {}", e),
                        )),
                    ))
                })?)
            } else {
                Ok(AdmissionResponse::from(req))
            }
        }
    }

    // Test validating handler
    struct TestValidator;

    #[async_trait::async_trait]
    impl ValidatingAdmissionHandler for TestValidator {
        type Err = HandlerError;

        fn name(&self) -> &'static str {
            "TestValidator"
        }

        async fn validate(
            &self,
            req: &AdmissionRequest<DynamicObject>,
        ) -> Result<AdmissionResponse, Self::Err> {
            // Reject resources with name "reject-me"
            if req.object.as_ref().and_then(|o| o.metadata.name.as_deref()) == Some("reject-me") {
                Ok(AdmissionResponse::from(req).deny("Resource name 'reject-me' is not allowed"))
            } else {
                Ok(AdmissionResponse::from(req))
            }
        }
    }

    fn create_test_request(name: &str) -> AdmissionRequest<DynamicObject> {
        // Create a minimal test request by deserializing from JSON
        let review_json = json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "test-uid",
                "kind": {"group": "", "version": "v1", "kind": "Pod"},
                "resource": {"group": "", "version": "v1", "resource": "pods"},
                "name": name,
                "namespace": "default",
                "operation": "CREATE",
                "userInfo": {},
                "object": {
                    "apiVersion": "v1",
                    "kind": "Pod",
                    "metadata": {
                        "name": name,
                        "namespace": "default"
                    },
                    "spec": {
                        "containers": []
                    }
                }
            }
        });

        let review: AdmissionReview<DynamicObject> = serde_json::from_value(review_json).unwrap();
        review.try_into().unwrap()
    }

    #[tokio::test]
    async fn test_mutating_handler_accepts() {
        let handler = TestMutator;
        let req = create_test_request("test-pod");

        let resp = handler.mutate(&req).await.unwrap();
        assert!(resp.allowed);
        assert!(resp.patch.is_some(), "Should have patches for test-pod");
    }

    #[tokio::test]
    async fn test_mutating_handler_no_patch() {
        let handler = TestMutator;
        let req = create_test_request("other-pod");

        let resp = handler.mutate(&req).await.unwrap();
        assert!(resp.allowed);
        assert!(resp.patch.is_none(), "Should not have patches for other-pod");
    }

    #[tokio::test]
    async fn test_validating_handler_accepts() {
        let handler = TestValidator;
        let req = create_test_request("allowed-pod");

        let resp = handler.validate(&req).await.unwrap();
        assert!(resp.allowed);
    }

    #[tokio::test]
    async fn test_validating_handler_denies() {
        let handler = TestValidator;
        let req = create_test_request("reject-me");

        let resp = handler.validate(&req).await.unwrap();
        assert!(!resp.allowed, "Should deny resource named 'reject-me'");
    }

    #[tokio::test]
    async fn test_merge_responses() {
        let req1 = create_test_request("test");
        let req2 = create_test_request("test");
        
        let resp1 = AdmissionResponse::from(&req1);
        let resp2 = AdmissionResponse::from(&req2).deny("test denial");
        
        let merged = merge_responses(resp1, resp2);
        assert!(!merged.allowed, "Merged response should deny when second response denies");
    }
}
