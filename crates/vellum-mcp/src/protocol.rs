// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! JSON-RPC 2.0 messages and the per-request metadata of MCP 2026-07-28.
//!
//! The protocol is stateless: every request carries its protocol version
//! and client capabilities in `params._meta`, so nothing here depends on a
//! connection or session.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The protocol revision this server implements.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// The handshake-based revision still accepted for clients that have not
/// moved to the stateless protocol yet (decision D86).
pub const LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";

/// Every revision this server answers, newest first.
pub const SUPPORTED_VERSIONS: [&str; 2] = [PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION];

/// `_meta` key: protocol version of a request.
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// `_meta` key: client name and version.
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
/// `_meta` key: client capabilities for one request.
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// `_meta` key: server name and version on every result.
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
/// `_meta` key: the subscription a notification belongs to.
pub const META_SUBSCRIPTION_ID: &str = "io.modelcontextprotocol/subscriptionId";
/// `_meta` key: W3C trace context.
pub const META_TRACEPARENT: &str = "traceparent";
/// `_meta` key: W3C trace state.
pub const META_TRACESTATE: &str = "tracestate";

/// Standard JSON-RPC and MCP error codes.
pub mod code {
    /// Invalid JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// Not a valid JSON-RPC request.
    pub const INVALID_REQUEST: i64 = -32600;
    /// Unknown method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid parameters, including an unknown resource.
    pub const INVALID_PARAMS: i64 = -32602;
    /// A server fault.
    pub const INTERNAL_ERROR: i64 = -32603;
    /// HTTP headers disagree with the body.
    pub const HEADER_MISMATCH: i64 = -32020;
    /// The request needs a client capability it did not declare.
    pub const MISSING_REQUIRED_CLIENT_CAPABILITY: i64 = -32021;
    /// The requested protocol version is not supported.
    pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
    /// Vellum: the token lacks a scope the operation needs. Allocated
    /// outside the JSON-RPC reserved range, as the specification asks.
    pub const INSUFFICIENT_SCOPE: i64 = -31003;
    /// Vellum: the caller is not authenticated.
    pub const UNAUTHENTICATED: i64 = -31001;
    /// Vellum: the per-token rate limit is exhausted.
    pub const RATE_LIMITED: i64 = -31029;
}

/// A JSON-RPC request or notification id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// A numeric id.
    Number(i64),
    /// A string id.
    String(String),
}

impl RequestId {
    /// The id as JSON.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Number(n) => json!(n),
            Self::String(s) => json!(s),
        }
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::String(s) => f.write_str(s),
        }
    }
}

/// An incoming message: a request (with an id) or a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    /// The id; `None` for a notification.
    pub id: Option<RequestId>,
    /// The method name.
    pub method: String,
    /// The parameters object (empty when absent).
    pub params: Map<String, Value>,
}

impl Incoming {
    /// Parses one JSON-RPC message.
    ///
    /// # Errors
    ///
    /// Returns an [`RpcError`] with `PARSE_ERROR` or `INVALID_REQUEST`, and
    /// the id when one could be read.
    pub fn parse(text: &str) -> Result<Self, (Option<RequestId>, RpcError)> {
        let value: Value = serde_json::from_str(text).map_err(|e| {
            (
                None,
                RpcError::new(code::PARSE_ERROR, format!("parse error: {e}")),
            )
        })?;
        Self::from_value(value)
    }

    /// Validates an already-parsed JSON value.
    ///
    /// # Errors
    ///
    /// Returns `INVALID_REQUEST` when the value is not a request or
    /// notification.
    pub fn from_value(value: Value) -> Result<Self, (Option<RequestId>, RpcError)> {
        let Value::Object(mut obj) = value else {
            return Err((None, RpcError::invalid_request("message must be an object")));
        };
        let id = match obj.remove("id") {
            None => None,
            Some(Value::Number(n)) => match n.as_i64() {
                Some(n) => Some(RequestId::Number(n)),
                None => {
                    return Err((
                        None,
                        RpcError::invalid_request("id must be an integer or string"),
                    ));
                }
            },
            Some(Value::String(s)) => Some(RequestId::String(s)),
            Some(_) => {
                return Err((
                    None,
                    RpcError::invalid_request("id must be an integer or string"),
                ));
            }
        };
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err((id, RpcError::invalid_request("jsonrpc must be \"2.0\"")));
        }
        if obj.contains_key("result") || obj.contains_key("error") {
            return Err((
                id,
                RpcError::invalid_request("clients must not send responses"),
            ));
        }
        let Some(Value::String(method)) = obj.remove("method") else {
            return Err((id, RpcError::invalid_request("method must be a string")));
        };
        let params = match obj.remove("params") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(p)) => p,
            Some(_) => return Err((id, RpcError::invalid_request("params must be an object"))),
        };
        Ok(Self { id, method, params })
    }

    /// The `_meta` object of the parameters, if any.
    #[must_use]
    pub fn meta(&self) -> Option<&Map<String, Value>> {
        self.params.get("_meta").and_then(Value::as_object)
    }

    /// A string parameter.
    #[must_use]
    pub fn str_param(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(Value::as_str)
    }
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message} ({code})")]
pub struct RpcError {
    /// The error code.
    pub code: i64,
    /// A short description.
    pub message: String,
    /// Structured detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Box<Value>>,
}

impl RpcError {
    /// An error with a code and message.
    #[must_use]
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Adds structured data.
    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(Box::new(data));
        self
    }

    /// `-32600 Invalid request`.
    #[must_use]
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(code::INVALID_REQUEST, message)
    }

    /// `-32602 Invalid params`.
    #[must_use]
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(code::INVALID_PARAMS, message)
    }

    /// `-32601 Method not found`.
    #[must_use]
    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            code::METHOD_NOT_FOUND,
            format!("method not found: {method}"),
        )
    }

    /// `-32603 Internal error`, without leaking detail.
    #[must_use]
    pub fn internal() -> Self {
        Self::new(code::INTERNAL_ERROR, "internal error")
    }

    /// `-32022 Unsupported protocol version`.
    #[must_use]
    pub fn unsupported_version(requested: &str) -> Self {
        Self::new(
            code::UNSUPPORTED_PROTOCOL_VERSION,
            "Unsupported protocol version",
        )
        .with_data(json!({ "supported": SUPPORTED_VERSIONS, "requested": requested }))
    }

    /// `-32020 Header mismatch`.
    #[must_use]
    pub fn header_mismatch(message: impl Into<String>) -> Self {
        Self::new(
            code::HEADER_MISMATCH,
            format!("Header mismatch: {}", message.into()),
        )
    }
}

/// A message the server writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// A successful response.
    Result {
        /// The request id.
        id: RequestId,
        /// The result object, including `resultType`.
        result: Value,
    },
    /// An error response; the id is absent when it could not be read.
    Error {
        /// The request id.
        id: Option<RequestId>,
        /// The error.
        error: RpcError,
    },
    /// A notification.
    Notification {
        /// The method.
        method: String,
        /// The parameters.
        params: Value,
    },
}

impl Outgoing {
    /// The JSON form of the message.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Result { id, result } => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Self::Error {
                id: Some(id),
                error,
            } => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
            Self::Error { id: None, error } => json!({ "jsonrpc": "2.0", "error": error }),
            Self::Notification { method, params } => {
                json!({ "jsonrpc": "2.0", "method": method, "params": params })
            }
        }
    }

    /// One line of JSON with no embedded newline, for stdio framing.
    #[must_use]
    pub fn to_line(&self) -> String {
        // serde_json never emits raw newlines in compact form.
        self.to_value().to_string()
    }
}

/// The client's declared identity, for display and audit only.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClientInfo {
    /// Client name.
    pub name: String,
    /// Client version.
    #[serde(default)]
    pub version: String,
}

/// The protocol fields of one request, read from `_meta`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestMeta {
    /// The protocol version of this request.
    pub protocol_version: String,
    /// The client capabilities for this request.
    pub client_capabilities: Map<String, Value>,
    /// The self-reported client identity.
    pub client_info: Option<ClientInfo>,
    /// W3C `traceparent`, when the client sent one.
    pub traceparent: Option<String>,
    /// W3C `tracestate`, when the client sent one.
    pub tracestate: Option<String>,
    /// The progress token, when the client asked for progress.
    pub progress_token: Option<Value>,
}

impl RequestMeta {
    /// Reads and validates the per-request fields.
    ///
    /// `fallback_version` is the version established out of band (the
    /// `MCP-Protocol-Version` header or a legacy `initialize`), used only
    /// when the request is a legacy one without `_meta` fields.
    ///
    /// # Errors
    ///
    /// `-32602` when a required field is missing or malformed, and
    /// `-32022` when the version is not supported.
    pub fn read(msg: &Incoming, fallback_version: Option<&str>) -> Result<Self, RpcError> {
        let empty = Map::new();
        let meta = msg.meta().unwrap_or(&empty);
        let version = match meta.get(META_PROTOCOL_VERSION) {
            Some(Value::String(v)) => v.clone(),
            Some(_) => {
                return Err(RpcError::invalid_params(
                    "_meta protocolVersion must be a string",
                ));
            }
            None => match fallback_version {
                Some(v) if v != PROTOCOL_VERSION => v.to_owned(),
                _ => {
                    return Err(RpcError::invalid_params(format!(
                        "missing _meta[\"{META_PROTOCOL_VERSION}\"]"
                    )));
                }
            },
        };
        if !SUPPORTED_VERSIONS.contains(&version.as_str()) {
            return Err(RpcError::unsupported_version(&version));
        }
        let modern = version == PROTOCOL_VERSION;
        let client_capabilities = match meta.get(META_CLIENT_CAPABILITIES) {
            Some(Value::Object(c)) => c.clone(),
            Some(_) => {
                return Err(RpcError::invalid_params(
                    "_meta clientCapabilities must be an object",
                ));
            }
            None if modern => {
                return Err(RpcError::invalid_params(format!(
                    "missing _meta[\"{META_CLIENT_CAPABILITIES}\"]"
                )));
            }
            None => Map::new(),
        };
        let client_info = meta
            .get(META_CLIENT_INFO)
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let text = |key: &str| meta.get(key).and_then(Value::as_str).map(str::to_owned);
        Ok(Self {
            protocol_version: version,
            client_capabilities,
            client_info,
            traceparent: text(META_TRACEPARENT),
            tracestate: text(META_TRACESTATE),
            progress_token: meta.get("progressToken").cloned(),
        })
    }

    /// Whether the request uses the stateless revision.
    #[must_use]
    pub fn is_modern(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
    }
}

/// Builds a complete result: adds `resultType` and the server identity.
#[must_use]
pub fn complete(mut body: Map<String, Value>, server_name: &str, server_version: &str) -> Value {
    let _ = body.insert("resultType".to_owned(), json!("complete"));
    let meta = body.entry("_meta").or_insert_with(|| json!({}));
    if let Value::Object(meta) = meta {
        let _ = meta.insert(
            META_SERVER_INFO.to_owned(),
            json!({ "name": server_name, "version": server_version }),
        );
    }
    Value::Object(body)
}

/// Turns a JSON value that is known to be an object into a map.
#[must_use]
pub fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            let _ = map.insert("value".to_owned(), other);
            map
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modern(method: &str) -> String {
        json!({
            "jsonrpc": "2.0", "id": 1, "method": method,
            "params": {"_meta": {
                META_PROTOCOL_VERSION: PROTOCOL_VERSION,
                META_CLIENT_CAPABILITIES: {},
                "traceparent": "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01"
            }}
        })
        .to_string()
    }

    #[test]
    fn parses_a_modern_request() -> Result<(), String> {
        let msg = Incoming::parse(&modern("tools/list")).map_err(|e| e.1.message)?;
        assert_eq!(msg.id, Some(RequestId::Number(1)));
        let meta = RequestMeta::read(&msg, None).map_err(|e| e.message)?;
        assert!(meta.is_modern());
        assert!(meta.traceparent.is_some());
        Ok(())
    }

    #[test]
    fn rejects_missing_meta() {
        let msg = Incoming {
            id: Some(RequestId::Number(1)),
            method: "tools/list".into(),
            params: Map::new(),
        };
        let err = RequestMeta::read(&msg, None).err();
        assert_eq!(err.map(|e| e.code), Some(code::INVALID_PARAMS));
    }

    #[test]
    fn rejects_unknown_versions_with_the_supported_list() {
        let mut msg = Incoming::parse(&modern("tools/list")).ok();
        if let Some(Value::Object(meta)) = msg.as_mut().and_then(|m| m.params.get_mut("_meta")) {
            let _ = meta.insert(META_PROTOCOL_VERSION.into(), json!("1900-01-01"));
        }
        let err = msg.and_then(|m| RequestMeta::read(&m, None).err());
        assert_eq!(
            err.as_ref().map(|e| e.code),
            Some(code::UNSUPPORTED_PROTOCOL_VERSION)
        );
        assert_eq!(
            err.and_then(|e| e.data).map(|d| d["requested"].clone()),
            Some(json!("1900-01-01"))
        );
    }

    #[test]
    fn legacy_requests_use_the_fallback_version() -> Result<(), String> {
        let msg = Incoming::parse(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#)
            .map_err(|e| e.1.message)?;
        let meta = RequestMeta::read(&msg, Some(LEGACY_PROTOCOL_VERSION)).map_err(|e| e.message)?;
        assert!(!meta.is_modern());
        Ok(())
    }

    #[test]
    fn rejects_malformed_messages() {
        for bad in [
            "[]",
            "{\"jsonrpc\":\"1.0\",\"method\":\"x\"}",
            "{\"jsonrpc\":\"2.0\",\"id\":null,\"method\":\"x\"}",
            "nope",
        ] {
            assert!(Incoming::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn complete_results_carry_result_type_and_server_info() {
        let v = complete(Map::new(), "vellum", "1.0.0");
        assert_eq!(v["resultType"], "complete");
        assert_eq!(v["_meta"][META_SERVER_INFO]["name"], "vellum");
    }
}
