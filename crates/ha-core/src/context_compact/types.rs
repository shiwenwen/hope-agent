// ── Types ──

use serde::Serialize;

// ── Token Estimate Calibrator ──

/// Calibrates token estimates using actual API usage feedback.
/// Uses exponential moving average (EMA) for smooth adaptation.
#[derive(Debug, Clone)]
pub struct TokenEstimateCalibrator {
    #[allow(dead_code)]
    calibration_factor: f64,
    #[allow(dead_code)]
    sample_count: u32,
}

impl TokenEstimateCalibrator {
    pub fn new() -> Self {
        Self {
            calibration_factor: 1.0,
            sample_count: 0,
        }
    }

    /// Update calibration factor with actual token count from API response.
    #[allow(dead_code)]
    pub fn update(&mut self, estimated: u32, actual: u32) {
        if estimated == 0 || actual == 0 {
            return;
        }
        let ratio = actual as f64 / estimated as f64;
        // EMA with α=0.3 (recent values weighted more)
        self.calibration_factor = self.calibration_factor * 0.7 + ratio * 0.3;
        self.sample_count += 1;
    }

    /// Apply calibration to a raw estimate.
    #[allow(dead_code)]
    pub fn calibrated_estimate(&self, raw_estimate: u32) -> u32 {
        (raw_estimate as f64 * self.calibration_factor) as u32
    }
}

impl Default for TokenEstimateCalibrator {
    fn default() -> Self {
        Self::new()
    }
}

// ── Compact Result ──

/// Result of a compaction operation, emitted as frontend event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactResult {
    /// Which tier was applied (0=no-op, 1/2/3/4)
    pub tier_applied: u8,
    /// Estimated tokens before compaction
    pub tokens_before: u32,
    /// Estimated tokens after compaction
    pub tokens_after: u32,
    /// Number of messages affected
    pub messages_affected: usize,
    /// Human-readable description
    pub description: String,
    /// Detailed breakdown
    pub details: Option<CompactDetails>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactDetails {
    pub tool_results_truncated: usize,
    pub tool_results_soft_trimmed: usize,
    pub tool_results_hard_cleared: usize,
    pub messages_summarized: usize,
    pub summary_tokens: Option<u32>,
}

/// Result of a prune operation.
pub struct PruneResult {
    pub soft_trimmed: usize,
    pub hard_cleared: usize,
    pub chars_freed: usize,
}

/// Result of splitting messages for summarization.
pub struct SummarizationSplit {
    pub summarizable: Vec<serde_json::Value>,
    pub preserved: Vec<serde_json::Value>,
    pub preserved_start_index: usize,
}

/// Information about a tool result found in a message.
pub(super) struct ToolResultInfo {
    /// Index in the messages array
    pub(super) msg_index: usize,
    /// Tool name (if extractable)
    #[allow(dead_code)]
    pub(super) tool_name: Option<String>,
    /// Content text length
    pub(super) content_chars: usize,
}
