use kube::Client;

/// Shared handler context
#[derive(Clone)]
pub struct Context<T = ()>
where
    T: Clone,
{
    /// Kube client
    pub client: Client,
    /// User defined data type
    pub data: T,
}

impl<T> From<(Client, T)> for Context<T>
where
    T: Clone,
{
    fn from((client, data): (Client, T)) -> Self {
        Self { client, data }
    }
}

impl From<Client> for Context {
    fn from(client: Client) -> Self {
        Self { client, data: () }
    }
}
