//! prost-generated types for Feishu's WebSocket framing protocol.
//!
//! Schema lives at `crates/ha-core/proto/pbbp2.proto`; the generated code is
//! produced by `build.rs` and lands in `OUT_DIR/pbbp2.rs`. Re-export the inner
//! `pbbp2::*` items at the module root so callers write `proto::Frame` instead
//! of `proto::pbbp2::Frame`.
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/pbbp2.rs"));
