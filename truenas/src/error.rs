use crate::types::RequestIdBuf;
use serde_json::value::RawValue;
use std::error::Error as StdError;
use tokio::sync::oneshot;

#[derive(thiserror::Error, Debug)]
pub enum Error {
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
    #[error("timeout")]
    Timeout,
    #[error("closed")]
    Closed,
    #[error(transparent)]
    TokioOneshotRecv(#[from] oneshot::error::RecvError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("transport: {0}")]
    Transport(#[from] Box<dyn StdError + Send + Sync>),
}

impl Error {
    pub fn transport_err<E>(e: E) -> Error
    where
        E: StdError + Send + Sync + 'static,
    {
        Error::Transport(Box::new(e))
    }
    pub fn tokiosend_err<E>(e: E) -> Error
    where
        E: StdError + Send + Sync + 'static,
    {
        Error::TokioSend(e.to_string())
    }
}
