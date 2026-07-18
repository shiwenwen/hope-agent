//! `image_generate` chat tool — thin front-end over the unified media-gen
//! stack (`crate::media_gen`): parses tool args, loads reference images,
//! delegates candidate resolution / failover / accounting to
//! `media_gen::execute_image`, and shapes the `__MEDIA_ITEMS__` result.

mod generate;
mod output;

pub(crate) use generate::tool_image_generate;
