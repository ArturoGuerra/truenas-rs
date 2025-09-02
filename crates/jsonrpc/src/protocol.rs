use std::{collections::HashMap, fmt};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

// Spec: https://www.jsonrpc.org/specification

// --- Misc ---

// JSONRPC 2.0 Params (https://www.jsonrpc.org/specification#parameter_structures)
#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum Params {
    Null,
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
}

impl Params {
    pub fn is_null(&self) -> bool {
        matches!(*self, Params::Null)
    }

    pub fn is_array(&self) -> bool {
        matches!(*self, Params::Array(_))
    }

    pub fn is_object(&self) -> bool {
        matches!(*self, Params::Object(_))
    }
}

// JSONRPC 2.0 Error Object (https://www.jsonrpc.org/specification#error_object)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Error {
    code: i64,
    message: String,
    data: Value,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "code: {} message: {} data: {:?}",
            self.code, self.message, self.data
        )
    }
}

// --- Requests ---

// JSONRPC 2.0 Request Object (https://www.jsonrpc.org/specification#request_object)
#[derive(Serialize, Deserialize, Debug)]
pub struct Request {
    jsonrpc: String, // This should always be 2.0
    id: String,
    method: String,
    params: Params,
}

// --- Bi-Directional ---

// JSONRPC 2.0 Notification Object, same as the request object but without an ID field.
// (https://www.jsonrpc.org/specification#notification)
#[derive(Serialize, Deserialize, Debug)]
pub struct Notification {
    jsonrpc: String, // This should always be 2.0
    method: String,
    params: Params,
}

// --- Responses ---

// JSONRPC 2.0 Response Object (https://www.jsonrpc.org/specification#response_object)
#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum RpcResponse<R> {
    Ok {
        jsonrpc: String,
        result: R,
        id: String,
    },
    Err {
        jsonrpc: String,
        error: Error,
        id: Option<String>,
    },
}

// Response wrapper for ease of use.
#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum Response<R> {
    Response(RpcResponse<R>),
    Notification(Notification),
}
