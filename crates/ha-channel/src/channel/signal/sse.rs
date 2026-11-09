use anyhow::{ensure, Result};

// Attachment bytes use deferred media; bound text and attachment metadata.
const MAX_BUFFERED_BYTES: usize = 1024 * 1024;

#[derive(Debug, PartialEq)]
pub(super) struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Decode complete UTF-8 lines, never individual HTTP chunks. One decoder per
/// connection: EOF drops unfinished events. No replay cursor is stored.
#[derive(Default)]
pub(super) struct SseDecoder {
    line: Vec<u8>,
    event: String,
    data: String,
    after_cr: bool,
    started: bool,
}

impl SseDecoder {
    pub fn push(&mut self, byte: u8) -> Result<Option<SseEvent>> {
        if std::mem::take(&mut self.after_cr) && byte == b'\n' {
            return Ok(None);
        }
        if byte == b'\r' || byte == b'\n' {
            self.after_cr = byte == b'\r';
            return self.finish_line();
        }
        ensure!(
            self.line.len() + self.event.len() + self.data.len() < MAX_BUFFERED_BYTES,
            "Signal SSE buffered event exceeds 1 MiB"
        );
        self.line.push(byte);
        Ok(None)
    }

    fn finish_line(&mut self) -> Result<Option<SseEvent>> {
        let line = std::str::from_utf8(&self.line)
            .map_err(|_| anyhow::anyhow!("Signal SSE line is not valid UTF-8"))?;
        let line = if !std::mem::replace(&mut self.started, true) {
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        let result = if line.is_empty() {
            let event = std::mem::take(&mut self.event);
            if self.data.is_empty() {
                None
            } else {
                // Even an empty data field contributes a newline.
                self.data.pop();
                Some(SseEvent {
                    event,
                    data: std::mem::take(&mut self.data),
                })
            }
        } else {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => {
                    self.event.clear();
                    self.event.push_str(value);
                }
                "data" => {
                    ensure!(
                        self.event.len() + self.data.len() + value.len() < MAX_BUFFERED_BYTES,
                        "Signal SSE buffered event exceeds 1 MiB"
                    );
                    self.data.push_str(value);
                    self.data.push('\n');
                }
                // Comments, id and retry do not enable daemon replay.
                _ => {}
            }
            None
        };
        self.line.clear();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(chunks: &[&[u8]]) -> Result<Vec<SseEvent>> {
        let mut decoder = SseDecoder::default();
        let mut events = Vec::new();
        for chunk in chunks {
            for byte in *chunk {
                if let Some(event) = decoder.push(*byte)? {
                    events.push(event);
                }
            }
        }
        Ok(events)
    }

    #[test]
    fn all_byte_splits_preserve_utf8_and_line_endings() {
        for ending in ["\n", "\r", "\r\n"] {
            let input = format!(
                "\u{feff}:keep-alive{ending}id: opaque-1{ending}event: receive{ending}data: {{\"message\":\"你好🦭\"}}{ending}{ending}"
            );
            for split in 0..=input.len() {
                assert_eq!(
                    decode(&[&input.as_bytes()[..split], &input.as_bytes()[split..]]).unwrap(),
                    vec![SseEvent {
                        event: "receive".into(),
                        data: "{\"message\":\"你好🦭\"}".into(),
                    }],
                    "ending {ending:?}, byte split {split}"
                );
            }
            let bytes = input.as_bytes().chunks(1).collect::<Vec<_>>();
            assert_eq!(decode(&bytes).unwrap().len(), 1);
        }
    }

    #[test]
    fn data_keeps_empty_lines_and_only_removes_one_ascii_space() {
        let events = decode(&[b"data\ndata:   hello\ndata:\ndata:\tworld\n\ndata:\n\n"]).unwrap();
        assert_eq!(events[0].data, "\n  hello\n\n\tworld");
        assert_eq!(events[1].data, "");
    }

    #[test]
    fn event_names_reset_and_do_not_trim_spaces() {
        let events =
            decode(&[b"event: receive \ndata: a\n\ndata: b\n\nevent: ignored\n\ndata: c\n\n"])
                .unwrap();
        assert_eq!(events[0].event, "receive ");
        assert_eq!(events[1].event, "");
        assert_eq!(events[2].event, "");
    }

    #[test]
    fn eof_and_new_connection_do_not_dispatch_partial_event() {
        assert!(decode(&[b"event: receive\ndata: incomplete\n"])
            .unwrap()
            .is_empty());
        assert!(decode(&[b"data: incomplete"]).unwrap().is_empty());
        assert!(decode(&[b"\n\n"]).unwrap().is_empty());
    }

    #[test]
    fn malformed_utf8_fails_without_echoing_payload() {
        let error = decode(&[b"data: private-\xff\n\n"])
            .unwrap_err()
            .to_string();
        assert_eq!(error, "Signal SSE line is not valid UTF-8");
    }

    #[test]
    fn unterminated_line_and_multiline_event_are_bounded() {
        let oversized = vec![b'x'; MAX_BUFFERED_BYTES + 1];
        assert!(decode(&[&oversized]).is_err());
        let line = format!("data: {}\n", "x".repeat(1024));
        let mut decoder = SseDecoder::default();
        let mut rejected = false;
        for _ in 0..1024 {
            for byte in line.bytes() {
                if decoder.push(byte).is_err() {
                    rejected = true;
                    break;
                }
            }
            if rejected {
                break;
            }
        }
        assert!(rejected);
        // The limit resets per event, not per HTTP chunk.
        let complete = format!("{line}\n");
        let chunks = vec![complete.as_bytes(); 1024];
        assert_eq!(decode(&chunks).unwrap().len(), 1024);
    }
}
