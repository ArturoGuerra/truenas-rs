use serde_json::Value;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("protocol error id: {id:?} code: {code:?} message: {message:?} data: {data:?}")]
    Protocol {
        id: Option<String>,
        code: i64,
        message: String,
        data: Value,
    },
}
