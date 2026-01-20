use std::fmt::Debug;
use std::hash::Hash;

use async_trait::async_trait;
use k8s_openapi::{ClusterResourceScope, NamespaceResourceScope};
use kube::api::{Api, DeleteParams, Patch, PatchParams};
use kube::{Client, Resource, ResourceExt};
use serde::{Serialize, de::DeserializeOwned};

/// Extensions for Kubernetes resource scopes
///
/// Provides methods to create appropriate `Api` instances for namespaced
/// or cluster-scoped resources.
pub trait ScopeExt<K>
where
    K: Resource<Scope = Self>,
{
    /// Creates an API client for the resource
    ///
    /// Returns a namespaced API if namespace is provided and the resource is namespaced,
    /// otherwise returns a cluster-wide API.
    fn api(client: Client, namespace: Option<impl AsRef<str>>) -> Api<K>;
}

impl<K> ScopeExt<K> for NamespaceResourceScope
where
    K: Resource<Scope = Self>,
    K::DynamicType: Default,
{
    fn api(client: Client, namespace: Option<impl AsRef<str>>) -> Api<K> {
        if let Some(namespace) = namespace {
            Api::namespaced(client, namespace.as_ref())
        } else {
            Api::all(client)
        }
    }
}

impl<K> ScopeExt<K> for ClusterResourceScope
where
    K: Resource<Scope = Self>,
    K::DynamicType: Default,
{
    fn api(client: Client, _: Option<impl AsRef<str>>) -> Api<K> {
        Api::all(client)
    }
}

#[async_trait]
pub trait ApiExt<K>
where
    K: Resource,
{
    async fn apply(self, client: &Client) -> kube::Result<K>;
    async fn apply_with_api(self, api: Api<K>) -> kube::Result<K>;
    async fn apply_if_not_exists(self, client: &Client) -> kube::Result<()>;
    async fn delete(self, client: &Client) -> kube::Result<()>;
    async fn exists(self, client: &Client) -> kube::Result<bool>;
}

#[async_trait]
impl<K, S> ApiExt<K> for K
where
    S: ScopeExt<K>,
    K: Resource<Scope = S> + Clone + Serialize + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Clone + Debug + Default + Hash + Unpin + Eq,
{
    async fn apply(self, client: &Client) -> kube::Result<K> {
        let api = K::Scope::api(client.clone(), self.namespace());
        self.apply_with_api(api).await
    }

    async fn apply_with_api(self, api: Api<K>) -> kube::Result<K> {
        api.patch(
            &self.name_unchecked(),
            &PatchParams::apply(env!("CARGO_PKG_NAME")),
            &Patch::Apply(self),
        )
        .await
    }

    async fn apply_if_not_exists(self, client: &Client) -> kube::Result<()> {
        let api = K::Scope::api(client.clone(), self.namespace());
        let name = self.name_unchecked();

        if let Err(kube::Error::Api(err)) = api.get(&name).await
            && err.is_not_found()
        {
            self.apply_with_api(api).await?;
        }

        Ok(())
    }

    async fn delete(self, client: &Client) -> kube::Result<()> {
        let api = K::Scope::api(client.clone(), self.namespace());
        let name = self.name_unchecked();
        let params = DeleteParams::default();
        api.delete(&name, &params).await?;
        Ok(())
    }

    async fn exists(self, client: &Client) -> kube::Result<bool> {
        let name = self.name_unchecked();
        let api = K::Scope::api(client.clone(), self.namespace());

        match api.get(&name).await {
            Ok(_) => Ok(true),
            Err(kube::Error::Api(err)) if err.is_not_found() => Ok(false),
            err => {
                err?;
                Ok(false)
            }
        }
    }
}
