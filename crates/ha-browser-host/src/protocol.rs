use std::io::{ErrorKind, Read, Write};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_NATIVE_MESSAGE_LEN: u32 = 1024 * 1024;

pub fn read_native_message<R: Read>(reader: &mut R) -> Result<Option<Value>> {
    let mut len_buf = [0u8; 4];
    let mut read = 0usize;
    while read < len_buf.len() {
        match reader.read(&mut len_buf[read..]) {
            Ok(0) if read == 0 => return Ok(None),
            Ok(0) => bail!("truncated native message length header"),
            Ok(n) => read += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e).context("reading native message length"),
        }
    }

    let len = u32::from_le_bytes(len_buf);
    if len == 0 {
        bail!("native message length must be greater than zero");
    }
    if len > MAX_NATIVE_MESSAGE_LEN {
        bail!(
            "native message length {} exceeds max {}",
            len,
            MAX_NATIVE_MESSAGE_LEN
        );
    }

    let mut payload = vec![0u8; len as usize];
    reader
        .read_exact(&mut payload)
        .context("reading native message payload")?;
    let value = serde_json::from_slice(&payload).context("decoding native message JSON")?;
    Ok(Some(value))
}

pub fn write_native_message<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    let payload = serde_json::to_vec(value).context("encoding native message JSON")?;
    if payload.len() > MAX_NATIVE_MESSAGE_LEN as usize {
        return Err(anyhow!(
            "native message length {} exceeds max {}",
            payload.len(),
            MAX_NATIVE_MESSAGE_LEN
        ));
    }

    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .context("writing native message length")?;
    writer
        .write_all(&payload)
        .context("writing native message payload")?;
    writer.flush().context("flushing native message")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_message_round_trips_json() {
        let input = json!({
            "id": "msg-1",
            "method": "hello",
            "payload": { "extensionVersion": "0.1.0" }
        });
        let mut bytes = Vec::new();
        write_native_message(&mut bytes, &input).expect("write frame");

        let decoded = read_native_message(&mut bytes.as_slice())
            .expect("read frame")
            .expect("message");
        assert_eq!(decoded, input);
    }

    #[test]
    fn eof_before_header_is_clean_end() {
        let decoded = read_native_message(&mut std::io::empty()).expect("read eof");
        assert!(decoded.is_none());
    }

    #[test]
    fn partial_header_is_error() {
        let err = read_native_message(&mut [1u8, 0].as_slice())
            .expect_err("partial header should fail")
            .to_string();
        assert!(err.contains("truncated native message length header"));
    }

    #[test]
    fn oversized_payload_is_rejected_before_allocation() {
        let len = (MAX_NATIVE_MESSAGE_LEN + 1).to_le_bytes();
        let err = read_native_message(&mut len.as_slice())
            .expect_err("oversized frame should fail")
            .to_string();
        assert!(err.contains("exceeds max"));
    }
}
