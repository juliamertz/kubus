use std::fmt::Debug;
use std::sync::Arc;

use kube::api::{Api, Patch, PatchParams};
use kube::{Resource, ResourceExt};
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use serde_json::json;

async fn patch_object<K>(api: &Api<K>, obj: Arc<K>, patch: serde_json::Value) -> kube::Result<()>
where
    K: Resource + Clone + Debug + Serialize + DeserializeOwned,
{
    api.patch::<K>(
        &obj.meta().name.clone().unwrap(),
        &PatchParams::default(),
        &Patch::Json(serde_json::from_value(patch).unwrap()),
    )
    .await?;

    Ok(())
}

pub async fn apply_finalizer<K>(api: &Api<K>, name: &str, obj: Arc<K>) -> kube::Result<()>
where
    K: Resource + Clone + Debug + Serialize + DeserializeOwned,
{
    let patch = if obj.finalizers().is_empty() {
        json!([
            { "op": "test", "path": "/metadata/finalizers", "value": null },
            { "op": "add", "path": "/metadata/finalizers", "value": [name] }
        ])
    } else {
        json!([
            { "op": "test", "path": "/metadata/finalizers", "value": obj.finalizers() },
            { "op": "add", "path": "/metadata/finalizers", "value": [name] }
        ])
    };

    patch_object(api, obj, patch).await
}

pub async fn remove_finalizer<K>(api: &Api<K>, name: &str, obj: Arc<K>) -> kube::Result<()>
where
    K: Resource + Clone + Debug + Serialize + DeserializeOwned,
{
    let Some(idx) = obj
        .finalizers()
        .iter()
        .enumerate()
        .find(|(_, n)| n == &name)
        .map(|(idx, _)| idx)
    else {
        return Ok(());
    };

    let finalizer_path = format!("/metadata/finalizers/{idx}");

    let patch = json!([
      { "op": "test", "path": finalizer_path, "value": name },
      { "op": "remove", "path": finalizer_path }
    ]);

    patch_object(api, obj, patch).await
}
