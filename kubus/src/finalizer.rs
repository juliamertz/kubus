// Modified from kube-rs source

use std::{fmt::Debug, sync::Arc};

use k8s_openapi::{
    serde::{Serialize, de::DeserializeOwned},
    serde_json::{from_value, json},
};
use kube::{
    Api, Resource, ResourceExt,
    api::{Patch, PatchParams},
};

use crate::EventType;

#[derive(Debug)]
struct FinalizerState {
    finalizer_index: Option<usize>,
    is_deleting: bool,
}

impl FinalizerState {
    fn for_object<K: Resource>(obj: &K, finalizer_name: &str) -> Self {
        Self {
            finalizer_index: obj
                .finalizers()
                .iter()
                .enumerate()
                .find(|(_, fin)| *fin == finalizer_name)
                .map(|(i, _)| i),
            is_deleting: obj.meta().deletion_timestamp.is_some(),
        }
    }
}

/// Update the finalizer of a kubernetes resource
///
/// The behaviour of this function depends on the `event_type` passed in
/// for `Apply` the finalizer is appended to the current list of finalizers
/// Otherwise for `Delete` events the finalizer is removed from the list
pub async fn update_finalizer<K>(
    api: &Api<K>,
    finalizer_name: &str,
    event_type: EventType,
    obj: Arc<K>,
) -> Result<(), crate::Error>
where
    K: Resource + Clone + Debug + Serialize + DeserializeOwned,
{
    match (
        event_type,
        FinalizerState::for_object(&*obj, finalizer_name),
    ) {
        (
            _,
            FinalizerState {
                finalizer_index: Some(_),
                is_deleting: false,
            },
        ) => Ok(()),

        (
            EventType::Delete,
            FinalizerState {
                finalizer_index: Some(i),
                is_deleting: true,
            },
        ) => {
            let name = obj.meta().name.clone().unwrap();
            let finalizer_path = format!("/metadata/finalizers/{i}");

            let patch = json!([
              { "op": "test", "path": finalizer_path, "value": finalizer_name },
              { "op": "remove", "path": finalizer_path }
            ]);

            tracing::info!("applying patch: {patch:?}");

            api.patch::<K>(
                &name,
                &PatchParams::default(),
                &Patch::Json(from_value(patch).unwrap()),
            )
            .await?;

            Ok(())
        }

        (
            EventType::Apply,
            FinalizerState {
                finalizer_index: None,
                is_deleting: false,
            },
        ) => {
            // Finalizer must be added before it's safe to run an `Apply` reconciliation
            let patch = if obj.finalizers().is_empty() {
                json!([
                    { "op": "test", "path": "/metadata/finalizers", "value": null },
                    { "op": "add", "path": "/metadata/finalizers", "value": [finalizer_name] }
                ])
            } else {
                json!([
                    { "op": "test", "path": "/metadata/finalizers", "value": obj.finalizers() },
                    { "op": "add", "path": "/metadata/finalizers", "value": finalizer_name }
                ])
            };

            tracing::info!("applying patch: {patch:?}");

            api.patch::<K>(
                obj.meta().name.as_deref().unwrap(),
                &PatchParams::default(),
                &Patch::Json(json_patch::Patch(from_value(patch).unwrap())),
            )
            .await?;
            Ok(())
        }

        other => {
            eprintln!("unhandled {other:?}");
            Ok(())
        }
    }
}
