use k8s_openapi::{ClusterResourceScope, NamespaceResourceScope};
use kube::{Api, Client, Resource};

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
