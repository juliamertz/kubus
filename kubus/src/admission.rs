use std::convert::Infallible;
use std::error::Error as StdError;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use kube::ResourceExt;
use kube::api::DynamicObject;
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview};
use tracing::{error, info, warn};

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

fn merge_responses(base: AdmissionResponse, other: AdmissionResponse) -> AdmissionResponse {
    use json_patch::Patch;
    use serde_json::*;

    if !other.allowed {
        return other;
    }

    let Some(base_patch) = base.patch.as_ref() else {
        return other;
    };
    let Some(other_patch) = other.patch.as_ref() else {
        return base;
    };

    let (Ok(base_patches), Ok(other_patches)) = (
        from_slice::<Vec<Value>>(base_patch),
        from_slice::<Vec<Value>>(other_patch),
    ) else {
        return base;
    };

    let combined = [base_patches, other_patches]
        .into_iter()
        .flatten()
        .collect::<Value>();

    let Ok(patch) = from_value::<Patch>(json!(combined)) else {
        return base;
    };

    other.with_patch(patch).unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HandlerError;
    use kube::core::admission::AdmissionRequest;
    use serde_json::json;

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
            use json_patch::jsonptr::PointerBuf;
            use json_patch::{AddOperation, Patch, PatchOperation};

            if req.object.as_ref().and_then(|o| o.metadata.name.as_deref()) == Some("test-pod") {
                let patches = Patch(vec![PatchOperation::Add(AddOperation {
                    path: PointerBuf::parse("/metadata/labels/test-label").unwrap(),
                    value: json!("test-value"),
                })]);
                Ok(AdmissionResponse::from(req)
                    .with_patch(patches)
                    .map_err(|e| {
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
            if req.object.as_ref().and_then(|o| o.metadata.name.as_deref()) == Some("reject-me") {
                Ok(AdmissionResponse::from(req).deny("Resource name 'reject-me' is not allowed"))
            } else {
                Ok(AdmissionResponse::from(req))
            }
        }
    }

    fn create_test_request(name: &str) -> AdmissionRequest<DynamicObject> {
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
        assert!(
            resp.patch.is_none(),
            "Should not have patches for other-pod"
        );
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
        assert!(
            !merged.allowed,
            "Merged response should deny when second response denies"
        );
    }
}
