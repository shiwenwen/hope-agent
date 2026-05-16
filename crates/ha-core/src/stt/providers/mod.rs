//! STT provider implementations.
//!
//! Phase 1 ships the OpenAI-compatible batch path (covers OpenAI Whisper,
//! gpt-4o-transcribe batch, Groq, DashScope/Qwen3-ASR via compatible mode,
//! StepFun, SiliconFlow, whisper.cpp server, faster-whisper-server, FunASR
//! + OpenAI wrapper, sherpa-onnx server). Streaming SSE and the WebSocket
//! providers (Deepgram, AssemblyAI, Azure, Volcengine, iFlytek) ship in
//! Phase 2 / Phase 6.

pub mod openai;
