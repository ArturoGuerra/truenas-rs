use thiserror::Error;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;

pub mod client;
pub mod conn;
pub mod messaging;
pub(crate) mod protocol;

#[derive(Error, Debug)]
pub enum Error {
    #[error("tungstenite error: {0}")]
    Tungstenite(TungsteniteError),
    #[error("channel send error: {0}")]
    SendError(tokio::sync::mpsc::error::SendError<messaging::Command>),
    #[error("oneshot channel recv error: {0}")]
    RecvError(tokio::sync::oneshot::error::RecvError),
    #[error("messaging: error: {0}")]
    MessagingError(messaging::Error),
    #[error("serde error: {0}")]
    SerdeJsonError(serde_json::Error),
}
