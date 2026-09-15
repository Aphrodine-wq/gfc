use std::os::unix::net::UnixStream;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

pub fn read_frame(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let len = u32::try_from(bytes.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}

pub fn ok(id: Value, result: Value) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

pub fn err(id: Value, code: i32, message: impl Into<String>) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(RpcError {
            code,
            message: message.into(),
        }),
    }
}

pub fn notify(method: &str, params: Value) -> RpcNotification {
    RpcNotification {
        jsonrpc: "2.0".into(),
        method: method.into(),
        params,
    }
}

pub fn connect(socket: &Path) -> std::io::Result<UnixStream> {
    UnixStream::connect(socket)
}

pub fn call(stream: &mut UnixStream, method: &str, params: Value) -> anyhow::Result<Value> {
    let req = RpcRequest {
        jsonrpc: "2.0".into(),
        id: json!(1),
        method: method.into(),
        params,
    };
    write_frame(stream, &serde_json::to_vec(&req)?)?;
    let frame = read_frame(stream)?;
    let resp: RpcResponse = serde_json::from_slice(&frame)?;
    if let Some(err) = resp.error {
        anyhow::bail!("{}: {}", err.code, err.message);
    }
    Ok(resp.result.unwrap_or(Value::Null))
}
