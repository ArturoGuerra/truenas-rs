/*
*  Messaging is the data abstraction used between the public api and the JSONRPC 2.0 protocol
*  state loop to improve ergonomics.
*/

use std::ops::Range;

use bytes::Bytes;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::value::RawValue;
pub use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};

// ---- Type aliases ----

pub type Result<T> = std::result::Result<T, Error>;
pub type RpcReply = oneshot::Sender<Result<RpcResultPayload>>;
pub type SubscriptionReady = oneshot::Sender<Result<()>>;
pub type SubscriptionSender = mpsc::UnboundedSender<Result<()>>;

// ---- Error definitions ----
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("transport: {0}")]
    Transport(String),
    #[error("protocol error code={code} message={message}")]
    Protocol {
        id: RequestId,
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("timeout")]
    Timeout,
    #[error("closed")]
    Closed,
}

#[derive(thiserror::Error, Debug)]
pub enum ParamConvError {
    #[error("params most be an array (positional) or object (named), got scaler")]
    NotArrayOrObject,
    #[error("serialization failure: {0}")]
    Serialization(serde_json::Error),
}

/*
* ---- Data types used in the messaging layer ----
*/

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct RequestId(String);

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<RequestId> for String {
    fn from(r: RequestId) -> Self {
        r.0
    }
}

/*
*
*/

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MethodName(String);
impl From<&str> for MethodName {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for MethodName {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<MethodName> for String {
    fn from(r: MethodName) -> Self {
        r.0
    }
}

impl AsRef<str> for MethodName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub struct ArrayParams(Vec<Value>);
impl Default for ArrayParams {
    fn default() -> Self {
        Self::new()
    }
}
impl ArrayParams {
    pub fn new() -> Self {
        Self(vec![])
    }

    pub fn insert<T: Serialize>(&mut self, v: T) -> std::result::Result<(), serde_json::Error> {
        self.0.push(serde_json::to_value(&v)?);
        Ok(())
    }
}

pub struct ObjectParams(Map<String, Value>);
impl Default for ObjectParams {
    fn default() -> Self {
        Self::new()
    }
}
impl ObjectParams {
    pub fn new() -> Self {
        Self(Map::new())
    }

    pub fn insert<T: Serialize>(
        &mut self,
        k: impl Into<String>,
        v: T,
    ) -> std::result::Result<(), serde_json::Error> {
        self.0.insert(k.into(), serde_json::to_value(&v)?);
        Ok(())
    }
}

// ---- Public Data structures used in the messaging layer. ----

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Params {
    Array(Vec<Value>),
    Object(Map<String, Value>),
}

impl Params {
    pub fn into_raw(self) -> serde_json::Result<Box<RawValue>> {
        serde_json::value::to_raw_value(&self)
    }
}

pub trait IntoParams {
    fn into_params(self) -> std::result::Result<Option<Params>, ParamConvError>;
}

impl IntoParams for Params {
    fn into_params(self) -> std::result::Result<Option<Params>, ParamConvError> {
        Ok(Some(self))
    }
}
impl IntoParams for () {
    fn into_params(self) -> std::result::Result<Option<Params>, ParamConvError> {
        Ok(None)
    }
}

impl IntoParams for Option<Params> {
    fn into_params(self) -> std::result::Result<Option<Params>, ParamConvError> {
        Ok(self)
    }
}

impl IntoParams for Value {
    fn into_params(self) -> std::result::Result<Option<Params>, ParamConvError> {
        Ok(Some(match self {
            Value::Array(a) => Params::Array(a),
            Value::Object(o) => Params::Object(o),
            _ => return Err(ParamConvError::NotArrayOrObject),
        }))
    }
}

impl IntoParams for ArrayParams {
    fn into_params(self) -> std::result::Result<Option<Params>, ParamConvError> {
        Ok(Some(Params::Array(self.0)))
    }
}

impl IntoParams for ObjectParams {
    fn into_params(self) -> std::result::Result<Option<Params>, ParamConvError> {
        Ok(Some(Params::Object(self.0)))
    }
}

impl<T: Serialize> IntoParams for &T {
    fn into_params(self) -> std::result::Result<Option<Params>, ParamConvError> {
        match serde_json::to_value(self).map_err(ParamConvError::Serialization)? {
            Value::Array(a) => Ok(Some(Params::Array(a))),
            Value::Object(o) => Ok(Some(Params::Object(o))),
            _ => Err(ParamConvError::NotArrayOrObject),
        }
    }
}

#[derive(Debug)]
pub enum WireIn {
    Recv(Bytes),
    Closed,
}

#[derive(Debug)]
pub enum WireOut {
    Send(Bytes),
    Close,
}

#[derive(Debug)]
pub struct RpcResultPayload {
    pub id: RequestId,
    pub result: JsonSlice,
}

pub enum Cmd {
    Call {
        id: RequestId,
        method: MethodName,
        params: Option<Params>,
        reply: RpcReply,
    },

    Notification {
        method: MethodName,
        params: Option<Params>,
    },

    Subscribe {
        method: MethodName,
        ready: SubscriptionReady,
    },

    Unsubscribe {
        id: MethodName,
    },
    Close,
}

#[derive(Debug)]
pub struct JsonSlice {
    buf: Bytes,
    range: Range<usize>,
}

impl JsonSlice {
    pub fn from_raw(buf: Bytes, root: &str, raw: &RawValue) -> Self {
        let raw = raw.get();
        let base = root.as_ptr() as usize;
        let offset = raw.as_ptr() as usize - base;
        Self {
            buf,
            range: offset..offset + raw.len(),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[self.range.clone()]
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.as_bytes()).unwrap()
    }

    pub fn into_bytes(self) -> Bytes {
        self.buf.slice(self.range)
    }

    pub fn decode<T: DeserializeOwned>(self) -> serde_json::Result<T> {
        serde_json::from_slice(&self.into_bytes())
    }
}
