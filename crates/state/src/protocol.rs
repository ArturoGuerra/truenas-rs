use crate::types::RequestId;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

// Spec: https://www.jsonrpc.org/specification

// JSONRPC 2.0 Request Object (https://www.jsonrpc.org/specification#request_object)
#[derive(Serialize, Debug)]
pub struct RpcRequest<'a> {
    pub jsonrpc: &'a str,
    pub id: RequestId,
    pub method: &'a str,
    pub params: Option<&'a RawValue>,
}

// JSONRPC 2.0 Notification Object, same as the request object but without an ID field.
// (https://www.jsonrpc.org/specification#notification)
#[derive(Serialize, Deserialize, Debug)]
pub struct Notification<'a> {
    pub jsonrpc: &'a str, // This should always be 2.0
    pub method: &'a str,
    pub params: Option<&'a RawValue>,
}

#[derive(Deserialize, Debug)]
pub struct RpcResponse<'a> {
    pub jsonrpc: &'a str,
    pub result: &'a RawValue,
    pub id: RequestId,
}

// JSONRPC 2.0 Error Object (https://www.jsonrpc.org/specification#error_object)
#[derive(Deserialize, Debug)]
pub struct RpcError<'a> {
    pub jsonrpc: &'a str,
    pub error: Error<'a>,
    pub id: Option<RequestId>,
}

// JSONRPC 2.0 Error Object (https://www.jsonrpc.org/specification#error_object)
#[derive(Deserialize, Clone, Debug)]
pub struct Error<'a> {
    pub code: i64,
    pub message: &'a str,
    pub data: &'a RawValue,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum Response<'a> {
    #[serde(borrow)]
    RpcResponse(RpcResponse<'a>),
    #[serde(borrow)]
    RpcError(RpcError<'a>),
    #[serde(borrow)]
    Notification(Notification<'a>),
}
