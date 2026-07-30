//! 媒体生成执行机器（provider adapters / execute / probe / catalog）。
//! 配置面（crud / resolve / wire 类型）在 `ha_core::media_gen`。

pub mod adapters;
pub mod catalog;
pub mod execute;
pub mod input;
pub mod overview;
pub mod probe;
pub mod voices;

pub use catalog::{media_provider_templates, MediaProviderTemplate, OPENAI_TTS_VOICES};
pub use execute::{
    execute_audio, execute_image, AudioRequest, ImageRequest, MediaExecOutcome, UsageMeta,
};
pub use ha_core::media_gen::*;
pub use input::{decode_data_url, infer_resolution, load_input_image, load_input_images};
pub use overview::{
    media_gen_overview, MediaCandidateOverview, MediaFunctionOverview, MediaGenOverview,
};
