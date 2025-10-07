use thiserror::Error;

/// Errors that can occur during operator execution
#[derive(Error, Debug)]
pub enum Error {
    /// Error during JSON serialization/deserialization
    #[error("SerializationError: {0}")]
    SerializationError(#[source] serde_json::Error),

    /// Error from the Kubernetes client
    #[error("Kube Error: {0}")]
    KubeError(#[from] kube::Error),

    /// Error returned from the event handler
    #[error("Handler Error: {0}")]
    Handler(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Errors that can occur during event handler execution
#[derive(Error, Debug)]
pub enum HandlerError {
    /// Error during JSON serialization/deserialization
    #[error("SerializationError: {0}")]
    SerializationError(#[source] serde_json::Error),

    /// Error from the Kubernetes client
    #[error("Kube Error: {0}")]
    KubeError(#[from] kube::Error),

    /// Error from the Kubus
    #[error("Kubus Error: {0}")]
    KubusError(#[from] Error),
}

/// Result type for Kubus operations
pub type Result<T, E = Error> = std::result::Result<T, E>;
