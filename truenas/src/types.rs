/*
*  Messaging is the data abstraction used between the public api and the JSONRPC 2.0 protocol
*  state loop to improve ergonomics.
*/

use crate::error::Error;
use bytes::Bytes;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::value::{Map, RawValue, Value};
use std::borrow::Borrow;
use std::ops::Range;
use tokio::sync::{
    broadcast,
    mpsc::{self, channel},
    oneshot,
};
use uuid::Uuid;

// ---- Type aliases ----

pub type Result<T> = std::result::Result<T, Error>;
pub type CmdTx = mpsc::UnboundedSender<Cmd>;
pub type CmdRx = mpsc::UnboundedReceiver<Cmd>;
pub type WireInTx = mpsc::UnboundedSender<WireIn>;
pub type WireInRx = mpsc::UnboundedReceiver<WireIn>;
pub type WireOutTx = mpsc::UnboundedSender<WireOut>;
pub type WireOutRx = mpsc::UnboundedReceiver<WireOut>;
pub type RpcReply = oneshot::Sender<Result<RpcResultPayload>>;
pub type SubscriptionReady = oneshot::Sender<SubscriptionRecv>;
pub type SubscriptionSender = broadcast::Sender<SubscriptionPayload>;
pub type SubscriptionRecv = broadcast::Receiver<SubscriptionPayload>;

// ---- Error definitions ----

#[derive(Debug)]
pub enum WireIn {
    Recv(Bytes),
    Ping,
    Pong,
    Closed,
}

#[derive(Debug)]
pub enum WireOut {
    Send(Bytes),
    Ping,
    Pong,
    Close,
}

/*
* ---- Data types used in the messaging layer ----
*/

#[derive(Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct RequestId(str);
impl RequestId {
    fn new(s: &str) -> &RequestId {
        unsafe { &*(s as *const str as *const RequestId) }
    }
}
impl ToOwned for RequestId {
    type Owned = RequestIdBuf;
    fn to_owned(&self) -> RequestIdBuf {
        RequestIdBuf(self.0.to_owned())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct RequestIdBuf(String);
impl RequestIdBuf {
    pub fn new(s: String) -> RequestIdBuf {
        RequestIdBuf(s)
    }
}
impl Borrow<RequestId> for RequestIdBuf {
    fn borrow(&self) -> &RequestId {
        RequestId::new(&self.0)
    }
}
impl AsRef<RequestId> for RequestIdBuf {
    fn as_ref(&self) -> &RequestId {
        RequestId::new(&self.0)
    }
}
impl From<String> for RequestIdBuf {
    fn from(s: String) -> RequestIdBuf {
        RequestIdBuf(s)
    }
}
impl From<Uuid> for RequestIdBuf {
    fn from(uuid: Uuid) -> RequestIdBuf {
        RequestIdBuf(uuid.to_string())
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MethodId(str);
impl MethodId {
    fn new(s: &str) -> &MethodId {
        unsafe { &*(s as *const str as *const MethodId) }
    }
}
impl ToOwned for MethodId {
    type Owned = MethodIdBuf;
    fn to_owned(&self) -> MethodIdBuf {
        MethodIdBuf(self.0.to_owned())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MethodIdBuf(String);
impl MethodIdBuf {
    pub fn new(s: String) -> MethodIdBuf {
        MethodIdBuf(s)
    }
}
impl Borrow<MethodId> for MethodIdBuf {
    fn borrow(&self) -> &MethodId {
        MethodId::new(&self.0)
    }
}
impl AsRef<MethodId> for MethodIdBuf {
    fn as_ref(&self) -> &MethodId {
        MethodId::new(&self.0)
    }
}
impl From<String> for MethodIdBuf {
    fn from(s: String) -> MethodIdBuf {
        MethodIdBuf(s)
    }
}

/*
*
*/

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
    fn into_params(self) -> std::result::Result<Option<Params>, Error>;
}

impl IntoParams for Params {
    fn into_params(self) -> std::result::Result<Option<Params>, Error> {
        Ok(Some(self))
    }
}
impl IntoParams for () {
    fn into_params(self) -> std::result::Result<Option<Params>, Error> {
        Ok(None)
    }
}

impl IntoParams for Option<Params> {
    fn into_params(self) -> std::result::Result<Option<Params>, Error> {
        Ok(self)
    }
}

impl IntoParams for Value {
    fn into_params(self) -> std::result::Result<Option<Params>, Error> {
        Ok(Some(match self {
            Value::Array(a) => Params::Array(a),
            Value::Object(o) => Params::Object(o),
            _ => return Err(Error::NotArrayOrObject),
        }))
    }
}

impl IntoParams for ArrayParams {
    fn into_params(self) -> std::result::Result<Option<Params>, Error> {
        Ok(Some(Params::Array(self.0)))
    }
}

impl IntoParams for ObjectParams {
    fn into_params(self) -> std::result::Result<Option<Params>, Error> {
        Ok(Some(Params::Object(self.0)))
    }
}

impl<T: Serialize> IntoParams for &T {
    fn into_params(self) -> std::result::Result<Option<Params>, Error> {
        match serde_json::to_value(self).map_err(Error::Serde)? {
            Value::Array(a) => Ok(Some(Params::Array(a))),
            Value::Object(o) => Ok(Some(Params::Object(o))),
            _ => Err(Error::NotArrayOrObject),
        }
    }
}

#[derive(Debug)]
pub struct RpcResultPayload {
    pub id: RequestIdBuf,
    pub result: JsonSlice,
}

#[derive(Debug)]
pub struct SubscriptionPayload(pub Option<JsonSlice>);

pub enum Cmd {
    Call {
        id: RequestIdBuf,
        method: MethodIdBuf,
        params: Option<Params>,
        reply: RpcReply,
    },

    Notification {
        method: MethodIdBuf,
        params: Option<Params>,
    },

    Subscribe {
        method: MethodIdBuf,
        ready: SubscriptionReady,
    },

    Unsubscribe {
        method: MethodIdBuf,
    },
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

    pub fn deserialize<'de, T>(&'de self) -> serde_json::Result<T>
    where
        T: Deserialize<'de>,
    {
        serde_json::from_slice(self.as_bytes())
    }

    pub fn deserialize_owned<T>(self) -> serde_json::Result<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.into_bytes())
    }
}
