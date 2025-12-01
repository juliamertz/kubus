use std::convert::Infallible;
use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use kube::ResourceExt;
use kube::api::DynamicObject;
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview};
use tracing::{error, info, warn};

#[async_trait]
pub trait AdmissionHandler {
    type Err;

    async fn handle(
        &self,
        res: AdmissionResponse,
        obj: &DynamicObject,
    ) -> Result<AdmissionResponse, Self::Err>;
}

pub(crate) fn create_route<E: Error + Send + Sync + 'static>(
    handlers: Vec<Box<dyn AdmissionHandler<Err = E> + Send + Sync + 'static>>,
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
                    error!("invalid request: {}", err.to_string());
                    return Ok(warp::reply::json(
                        &AdmissionResponse::invalid(err.to_string()).into_review(),
                    ));
                }
            };

            let mut res = AdmissionResponse::from(&req);

            if let Some(obj) = req.object {
                let name = obj.name_any();
                let kind = obj.types.clone().unwrap_or_default().kind;

                for handler in handlers.iter() {
                    match handler.handle(res.clone(), &obj).await {
                        Ok(handler_res) => {
                            res = handler_res;
                            info!("accepted: {:?} on {kind}/{name}", req.operation);
                        }
                        Err(err) => {
                            warn!("denied: {:?} on {kind}/{name} ({})", req.operation, err);
                            res = res.deny(err.to_string());
                            break;
                        }
                    }
                }
            };
            Ok(warp::reply::json(&res.into_review()))
        })
    }
}
