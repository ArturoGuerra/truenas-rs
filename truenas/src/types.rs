/*
*  Messaging is the data abstraction used between the public api and the JSONRPC 2.0 protocol
*  state loop to improve ergonomics.
*/

use crate::error::Error;
use bytes::Bytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::value::{Map, RawValue, Value};
use std::fmt::{self, Display};
use std::sync::Arc;
use std::{
    borrow::{Borrow, Cow},
    ops::Range,
    result::Result as StdResult,
};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::Duration,
};
use uuid::Uuid;

pub const IO_INTERNAL_EVENT_CAP: usize = 64;
pub const IO_WRITE_BUDGET: usize = 32; // Latency vs throughput
pub const CMD_CHANNEL_CAP: usize = 128;
pub const WIRE_OUT_CAP: usize = 256;
pub const WIRE_IN_CAP: usize = 256;
pub const IO_EVENT_CAP: usize = 8;
pub const IO_CTRL_CAP: usize = 4;
pub const STATE_EVENT_CAP: usize = 8;
pub const STATE_CTRL_CAP: usize = 4;
pub const OUTQ_BACKPREASSURE_THRESHOLD: usize = 192;
pub const OUTQ_CAP: usize = 256;
pub const JSONRPC_VERSION: &str = "2.0";

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct RequestId(u128);

impl RequestId {
    pub fn new() -> Self {
        RequestId(Uuid::new_v4().as_u128())
    }
}

impl Default for RequestId {
    fn default() -> Self {
        RequestId::new()
    }
}

impl From<Uuid> for RequestId {
    fn from(u: Uuid) -> RequestId {
        RequestId(u.as_u128())
    }
}

impl From<RequestId> for Uuid {
    fn from(r: RequestId) -> Uuid {
        Uuid::from_u128(r.0)
    }
}

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MethodId(str);

impl MethodId {
    pub fn new(s: &str) -> &MethodId {
        unsafe { &*(s as *const str as *const _ as *const MethodId) }
    }
}

impl ToOwned for MethodId {
    type Owned = MethodIdBuf;
    fn to_owned(&self) -> MethodIdBuf {
        MethodIdBuf(self.0.to_owned())
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
        MethodIdBuf::new(s)
    }
}

impl From<&str> for MethodIdBuf {
    fn from(s: &str) -> MethodIdBuf {
        MethodIdBuf::new(s.to_owned())
    }
}

// TODO: Refactor how things are origanized.

// ---- Type aliases ----

pub type Result<T> = StdResult<T, Error>;
pub type CmdTx = mpsc::Sender<Cmd>;
pub type CmdRx = mpsc::Receiver<Cmd>;
pub type SubscriptionReady = oneshot::Sender<SubscriptionRecv>;
pub type SubscriptionSender = broadcast::Sender<SubscriptionPayload>;
pub type SubscriptionRecv = broadcast::Receiver<SubscriptionPayload>;

/*
* ----  ----
*/

pub type WireInTx = mpsc::Sender<WireIn>;
pub type WireInRx = mpsc::Receiver<WireIn>;

#[derive(Debug)]
pub enum WireIn {
    Data(Bytes),
}

pub type WireOutTx = mpsc::Sender<WireOut>;
pub type WireOutRx = mpsc::Receiver<WireOut>;

#[derive(Debug)]
pub enum WireOut {
    Data(Bytes),
}

/*
*
*/
/// Zero-sized type that represents the JSON-RPC version "2.0".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JsonRpcVer;

impl Serialize for JsonRpcVer {
    fn serialize<S>(&self, ser: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ser.serialize_str(JSONRPC_VERSION)
    }
}

impl<'de> Deserialize<'de> for JsonRpcVer {
    fn deserialize<D>(de: D) -> StdResult<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = <&str>::deserialize(de)?;
        if s == JSONRPC_VERSION {
            Ok(JsonRpcVer)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected {:?}, got {:?}",
                JSONRPC_VERSION, s
            )))
        }
    }
}

/*
*
*/

#[derive(Debug, Clone)]
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

    pub fn insert<T: Serialize>(&mut self, v: T) -> StdResult<(), serde_json::Error> {
        self.0.push(serde_json::to_value(&v)?);
        Ok(())
    }
}

/*
*
*/

#[derive(Debug, Clone)]
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
    ) -> StdResult<(), serde_json::Error> {
        self.0.insert(k.into(), serde_json::to_value(&v)?);
        Ok(())
    }
}

/*
* ---- Public Data structures used in the messaging layer. ----
*/

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
    fn into_params(self) -> Result<Option<Params>>;
}

impl IntoParams for Params {
    fn into_params(self) -> Result<Option<Params>> {
        Ok(Some(self))
    }
}
impl IntoParams for () {
    fn into_params(self) -> Result<Option<Params>> {
        Ok(None)
    }
}

impl IntoParams for Option<Params> {
    fn into_params(self) -> Result<Option<Params>> {
        Ok(self)
    }
}

impl IntoParams for Value {
    fn into_params(self) -> Result<Option<Params>> {
        Ok(Some(match self {
            Value::Array(a) => Params::Array(a),
            Value::Object(o) => Params::Object(o),
            _ => return Err(Error::NotArrayOrObject),
        }))
    }
}

impl IntoParams for ArrayParams {
    fn into_params(self) -> Result<Option<Params>> {
        Ok(Some(Params::Array(self.0)))
    }
}

impl IntoParams for ObjectParams {
    fn into_params(self) -> Result<Option<Params>> {
        Ok(Some(Params::Object(self.0)))
    }
}

impl<T: Serialize> IntoParams for &T {
    fn into_params(self) -> Result<Option<Params>> {
        match serde_json::to_value(self).map_err(Error::Serde)? {
            Value::Array(a) => Ok(Some(Params::Array(a))),
            Value::Object(o) => Ok(Some(Params::Object(o))),
            _ => Err(Error::NotArrayOrObject),
        }
    }
}

/*
* ----  ----
*/
pub type RpcReply = oneshot::Sender<RpcResult>;

#[derive(Debug)]
pub struct RpcPayload {
    pub id: RequestId,
    pub result: JsonSlice,
}

#[derive(Debug)]
pub struct RpcError {
    pub id: RequestId,
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

pub type RpcResult = StdResult<RpcPayload, RpcError>;

/*
* ----  ----
*/

#[derive(Debug)]
pub struct SubscriptionPayload(pub Option<JsonSlice>);

/*
* ----  ----
*/

pub enum Cmd {
    Call {
        id: RequestId,
        method: MethodIdBuf,
        params: Option<Params>,
        reply: RpcReply,
        timeout: Duration,
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

/*
* ----  ----
*/

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
