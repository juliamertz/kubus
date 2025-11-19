use std::convert::Infallible;
use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use kube::ResourceExt;
use kube::api::DynamicObject;
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview};
use tracing::{error, info, warn};

use crate::Named;

#[async_trait]
pub trait AdmissionHandler<E> {
    async fn handle(
        &self,
        res: AdmissionResponse,
        obj: &DynamicObject,
    ) -> Result<AdmissionResponse, E>;
}

#[async_trait]
pub(crate) trait DynAdmissionHandler<E>: Send + Sync
where
    E: Error + Send + Sync + 'static,
{
    fn name(&self) -> &'static str;

    async fn handle(
        &self,
        res: AdmissionResponse,
        obj: &DynamicObject,
    ) -> Result<AdmissionResponse, E>;
}

pub(crate) struct AdmissionHandlerWrapper<E, H>
where
    E: Error + Sync + Send + 'static,
    H: AdmissionHandler<E> + Named + Sync + Send + 'static,
{
    _phantom: std::marker::PhantomData<(E, H)>,
}

impl<E, H> Default for AdmissionHandlerWrapper<E, H>
where
    E: Error + Sync + Send + 'static,
    H: AdmissionHandler<E> + Named + Sync + Send + 'static,
{
    fn default() -> Self {
        Self {
            _phantom: std::marker::PhantomData::default(),
        }
    }
}

#[async_trait]
impl<E, H> DynAdmissionHandler<E> for AdmissionHandlerWrapper<E, H>
where
    E: Error + Sync + Send + 'static,
    H: AdmissionHandler<E> + Named + Sync + Send + 'static,
{
    fn name(&self) -> &'static str {
        H::NAME
    }

    async fn handle(
        &self,
        res: AdmissionResponse,
        obj: &DynamicObject,
    ) -> Result<AdmissionResponse, E> {
        todo!()
    }
}

pub(crate) fn create_mutate_handler<E: Error + Send + Sync + 'static>(
    handlers: Vec<Box<dyn DynAdmissionHandler<E>>>,
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
