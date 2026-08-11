use std::collections::{HashMap, VecDeque};
use std::sync::RwLock;

use super::{ProviderFamily, RequestShape, TokenCount, TokenCountSource, TokenizerId};

const MAX_SAMPLES_PER_KEY: usize = 64;
const EMA_ALPHA: f64 = 0.2;
const MIN_FACTOR: f64 = 0.5;
const MAX_FACTOR: f64 = 4.0;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CalibrationKey {
    pub provider: ProviderFamily,
    pub model: String,
    pub request_shape: RequestShape,
    pub tokenizer_id: Option<TokenizerId>,
    pub tokenizer_registry_version: u32,
    pub has_media: bool,
}

#[derive(Debug, Clone)]
struct CalibrationBucket {
    center: f64,
    ratios: VecDeque<f64>,
}

impl Default for CalibrationBucket {
    fn default() -> Self {
        Self {
            center: 1.0,
            ratios: VecDeque::new(),
        }
    }
}

impl CalibrationBucket {
    fn observe(&mut self, estimated: u64, actual: u64) {
        if estimated == 0 || actual == 0 {
            return;
        }
        let ratio = (actual as f64 / estimated as f64).clamp(MIN_FACTOR, MAX_FACTOR);
        self.center = self.center * (1.0 - EMA_ALPHA) + ratio * EMA_ALPHA;
        if self.ratios.len() == MAX_SAMPLES_PER_KEY {
            self.ratios.pop_front();
        }
        self.ratios.push_back(ratio);
    }

    fn bounds(&self) -> (f64, f64, f64) {
        if self.ratios.is_empty() {
            return (1.0, 1.0, 1.0);
        }
        let mut values: Vec<f64> = self.ratios.iter().copied().collect();
        values.sort_by(f64::total_cmp);
        let lower = percentile(&values, 0.05).min(self.center);
        let upper = percentile(&values, 0.95).max(self.center);
        (lower, self.center, upper)
    }
}

fn percentile(values: &[f64], quantile: f64) -> f64 {
    let rank = (quantile.clamp(0.0, 1.0) * values.len() as f64)
        .ceil()
        .max(1.0) as usize;
    values[rank.saturating_sub(1).min(values.len() - 1)]
}

#[derive(Debug, Default)]
pub struct TokenCalibrationStore {
    buckets: RwLock<HashMap<CalibrationKey, CalibrationBucket>>,
}

impl TokenCalibrationStore {
    pub fn observe(&self, key: CalibrationKey, estimated: u64, actual: u64) {
        self.buckets
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .entry(key)
            .or_default()
            .observe(estimated, actual);
    }

    pub fn apply(&self, key: &CalibrationKey, mut count: TokenCount) -> TokenCount {
        let Some((lower, center, upper)) = self
            .buckets
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            .map(CalibrationBucket::bounds)
        else {
            return count;
        };
        let raw = count.estimated;
        let raw_lower = count.lower_bound;
        let raw_upper = count.upper_bound;
        count.estimated = scaled(raw, center);
        // Calibration refines observed bias but must not erase the base
        // estimator's uncertainty (especially heuristic/media bounds).
        count.lower_bound = scaled(raw_lower, lower).min(count.estimated);
        count.upper_bound = scaled(raw_upper, upper).max(count.estimated);
        count.source = match count.source {
            TokenCountSource::LocalTokenizer | TokenCountSource::CalibratedTokenizer => {
                TokenCountSource::CalibratedTokenizer
            }
            _ => TokenCountSource::CalibratedHeuristic,
        };
        count
    }
}

fn scaled(value: u64, factor: f64) -> u64 {
    (value as f64 * factor).ceil().clamp(0.0, u64::MAX as f64) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_accounting::{TokenBreakdown, TokenCountConfidence};

    #[test]
    fn calibration_keeps_lower_estimate_upper_in_order() {
        let store = TokenCalibrationStore::default();
        let key = CalibrationKey {
            provider: ProviderFamily::OpenAiResponses,
            model: "gpt-5".to_string(),
            request_shape: RequestShape::OpenAiResponses,
            tokenizer_id: Some(TokenizerId::O200kBase),
            tokenizer_registry_version: 1,
            has_media: false,
        };
        store.observe(key.clone(), 100, 150);
        let count = TokenCount::new(
            100,
            90,
            115,
            TokenCountSource::LocalTokenizer,
            TokenCountConfidence::High,
            Some(TokenizerId::O200kBase),
            1,
            RequestShape::OpenAiResponses,
            TokenBreakdown::default(),
            Vec::new(),
        );
        let count = store.apply(&key, count);
        assert!(count.lower_bound <= count.estimated);
        assert!(count.estimated <= count.upper_bound);
        assert_eq!(count.source, TokenCountSource::CalibratedTokenizer);
    }

    #[test]
    fn calibration_preserves_base_uncertainty() {
        let store = TokenCalibrationStore::default();
        let key = CalibrationKey {
            provider: ProviderFamily::Unknown,
            model: "custom".to_string(),
            request_shape: RequestShape::Json,
            tokenizer_id: None,
            tokenizer_registry_version: 1,
            has_media: true,
        };
        store.observe(key.clone(), 100, 100);
        let count = TokenCount::new(
            100,
            75,
            150,
            TokenCountSource::Heuristic,
            TokenCountConfidence::Low,
            None,
            1,
            RequestShape::Json,
            TokenBreakdown::default(),
            Vec::new(),
        );

        let count = store.apply(&key, count);

        assert!(count.lower_bound <= 75);
        assert!(count.upper_bound >= 150);
    }

    #[test]
    fn small_sample_percentiles_keep_both_tails() {
        let values = [0.5, 4.0];
        assert_eq!(percentile(&values, 0.05), 0.5);
        assert_eq!(percentile(&values, 0.95), 4.0);
    }
}
