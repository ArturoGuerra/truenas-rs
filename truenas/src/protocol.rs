use crate::types::{JsonRpcVer, MethodId, RequestId};
use serde::{Deserialize, Serialize, de::Error as CustomError};
use serde_json::{Value, value::RawValue};
use std::borrow::Cow;

// Spec: https://www.jsonrpc.org/specification

// JSONRPC 2.0 Request Object (https://www.jsonrpc.org/specification#request_object)
#[derive(Serialize, Debug)]
pub struct RpcRequest<'a> {
    pub jsonrpc: JsonRpcVer,
    pub id: Cow<'a, RequestId>,
    pub method: Cow<'a, MethodId>,
    pub params: Option<&'a RawValue>,
}

// JSONRPC 2.0 Notification Object, same as the request object but without an ID field.
// (https://www.jsonrpc.org/specification#notification)
#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Notification<'a> {
    pub jsonrpc: JsonRpcVer,
    #[serde(borrow)]
    pub method: Cow<'a, MethodId>,
    #[serde(borrow)]
    pub params: Option<&'a RawValue>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RpcResponse<'a> {
    #[expect(dead_code, reason = "Serde-only marker that’s never read in code")]
    pub jsonrpc: JsonRpcVer,
    #[serde(borrow)]
    pub result: &'a RawValue,
    #[serde(borrow)]
    pub id: Cow<'a, RequestId>,
}

// JSONRPC 2.0 Error Object (https://www.jsonrpc.org/specification#error_object)
#[derive(Debug)]
pub struct RpcError<'a> {
    #[expect(dead_code, reason = "Serde-only marker that’s never read in code")]
    pub jsonrpc: JsonRpcVer,
    pub error: Error<'a>,
    pub id: Cow<'a, RequestId>,
}

// JSONRPC 2.0 Error Object (https://www.jsonrpc.org/specification#error_object)
#[derive(Deserialize, Debug)]
pub struct Error<'a> {
    pub code: i64,
    #[expect(dead_code, reason = "Todo")]
    pub method: Cow<'a, MethodId>,
    #[serde(borrow)]
    pub message: Cow<'a, str>,
    #[serde(borrow)]
    pub data: Option<Cow<'a, RawValue>>,
}

#[derive(Debug)]
pub enum Response<'a> {
    RpcResponse(RpcResponse<'a>),
    RpcError(RpcError<'a>),
    Notification(Notification<'a>),
}

#[derive(Deserialize, Debug)]
pub struct ResponseAny<'a> {
    #[expect(dead_code, reason = "Serde-only marker that’s never read in code")]
    pub jsonrpc: JsonRpcVer,
    #[serde(borrow)]
    pub id: Option<Cow<'a, RequestId>>,
    #[serde(borrow)]
    pub result: Option<&'a RawValue>,
    #[serde(borrow)]
    pub error: Option<Error<'a>>,
    #[serde(borrow)]
    pub method: Option<Cow<'a, MethodId>>,
    #[serde(borrow)]
    pub params: Option<&'a RawValue>,
}

impl<'a> TryFrom<ResponseAny<'a>> for Response<'a> {
    type Error = serde_json::Error;
    fn try_from(any: ResponseAny<'a>) -> std::result::Result<Response<'a>, Self::Error> {
        match (any.id, any.result, any.error, any.method, any.params) {
            // result = ID, result
            (Some(id), Some(result), None, None, None) => Ok(Response::RpcResponse(RpcResponse {
                jsonrpc: JsonRpcVer,
                result,
                id,
            })), // error = ID, error
            (Some(id), None, Some(error), None, None) => Ok(Response::RpcError(RpcError {
                jsonrpc: JsonRpcVer,
                error,
                id,
            })),
            (None, None, None, Some(method), Some(params)) => {
                Ok(Response::Notification(Notification {
                    jsonrpc: JsonRpcVer,
                    method,
                    params: Some(params),
                }))
            }

            (None, None, None, Some(method), None) => Ok(Response::Notification(Notification {
                jsonrpc: JsonRpcVer,
                method,
                params: None,
            })),
            _ => Err(serde_json::Error::custom(
                "unknown or invalid JSON-RPC shape",
            )),
        }
    }
}
