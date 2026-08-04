//! Desktop pet configuration (`AppConfig.pet`).
//!
//! 仅 config.json wire 类型（`PetConfig` / `PetRef`）；宠物库、导入、活动
//! 投影等运行时类型留在 ha-core `pet/types.rs`。

use serde::{Deserialize, Serialize};

pub const BUILTIN_DEFAULT_PET_REF: &str = "builtin:hope-default";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PetRef(pub String);

impl Default for PetRef {
    fn default() -> Self {
        Self(BUILTIN_DEFAULT_PET_REF.to_string())
    }
}

impl PetRef {
    pub fn builtin_id(&self) -> Option<&str> {
        self.0.strip_prefix("builtin:")
    }

    pub fn custom_id(&self) -> Option<&str> {
        self.0.strip_prefix("custom:")
    }

    pub fn is_well_formed(&self) -> bool {
        self.builtin_id()
            .or_else(|| self.custom_id())
            .is_some_and(crate::paths::is_valid_pet_id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub selected_pet_ref: PetRef,
}
