use crate::types::RequestIdBuf;
use serde_json::value::RawValue;
use tokio::sync::oneshot;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("transport: {0}")]
    Transport(String),
    #[error("protocol error code={code} message={message}")]
    Protocol {
        id: RequestIdBuf,
        code: i64,
        message: String,
        data: Option<Box<RawValue>>,
    },
    #[error("params must be an array or object got a scalar")]
    NotArrayOrObject,
    #[error("tokio send error: {0}")]
    TokioSend(String),
    #[error(transparent)]
    TokioOneshotRecv(#[from] oneshot::error::RecvError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("timeout")]
    Timeout,
    #[error("closed")]
    Closed,
}
