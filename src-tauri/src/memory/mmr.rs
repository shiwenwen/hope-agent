use std::collections::HashSet;

/// MMR (Maximal Marginal Relevance) reranking.
/// Balances relevance with diversity to avoid redundant results.
///
/// Algorithm: iteratively selects the candidate that maximizes
///   MMR = λ * relevance - (1-λ) * max_similarity_to_already_selected
///
/// Text similarity uses Jaccard coefficient over token sets,
/// supporting CJK unigrams + bigrams for Chinese/Japanese/Korean text.

/// Tokenize text into a set of tokens for similarity comparison.
/// Handles ASCII words and CJK characters (unigrams + bigrams).
fn tokenize(text: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    let lower = text.to_lowercase();

    // ASCII words
    let mut word = String::new();
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            word.push(ch);
        } else {
            if word.len() > 1 {
                tokens.insert(word.clone());
            }
            word.clear();
        }
    }
    if word.len() > 1 {
        tokens.insert(word);
    }

    // CJK unigrams and bigrams
    let cjk_chars: Vec<char> = lower.chars().filter(|c| is_cjk(*c)).collect();
    for ch in &cjk_chars {
        tokens.insert(ch.to_string());
    }
    for pair in cjk_chars.windows(2) {
        tokens.insert(format!("{}{}", pair[0], pair[1]));
    }

    tokens
}

/// Check if a character is in a CJK Unicode block.
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}' |   // CJK Unified Ideographs
        '\u{3400}'..='\u{4DBF}' |   // CJK Extension A
        '\u{3040}'..='\u{309F}' |   // Hiragana
        '\u{30A0}'..='\u{30FF}' |   // Katakana
        '\u{AC00}'..='\u{D7AF}'     // Hangul Syllables
    )
}

/// Jaccard similarity between two token sets: |A ∩ B| / |A ∪ B|.
fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 { 0.0 } else { intersection as f32 / union as f32 }
}

/// MMR reranking: from `candidates`, iteratively select `k` results
/// balancing relevance with diversity.
///
/// - `candidates`: (id, relevance_score, content)
/// - `k`: number of results to return
/// - `lambda`: 0.0 = pure diversity, 1.0 = pure relevance
///
/// Returns (id, adjusted_score) pairs in selected order.
pub fn mmr_rerank(
    candidates: &[(i64, f32, &str)],
    k: usize,
    lambda: f32,
) -> Vec<(i64, f32)> {
    if candidates.is_empty() || k == 0 {
        return Vec::new();
    }

    let k = k.min(candidates.len());

    // Pre-tokenize all candidates
    let token_sets: Vec<HashSet<String>> = candidates.iter()
        .map(|(_, _, content)| tokenize(content))
        .collect();

    // Normalize relevance scores to [0, 1]
    let max_score = candidates.iter().map(|(_, s, _)| *s).fold(f32::NEG_INFINITY, f32::max);
    let min_score = candidates.iter().map(|(_, s, _)| *s).fold(f32::INFINITY, f32::min);
    let score_range = max_score - min_score;

    let normalized_scores: Vec<f32> = if score_range > 1e-10 {
        candidates.iter().map(|(_, s, _)| (s - min_score) / score_range).collect()
    } else {
        vec![1.0; candidates.len()]
    };

    let mut selected: Vec<usize> = Vec::with_capacity(k);
    let mut result: Vec<(i64, f32)> = Vec::with_capacity(k);
    let mut remaining: Vec<usize> = (0..candidates.len()).collect();

    for _ in 0..k {
        if remaining.is_empty() {
            break;
        }

        let mut best_idx_in_remaining = 0;
        let mut best_mmr = f32::NEG_INFINITY;

        for (ri, &ci) in remaining.iter().enumerate() {
            let relevance = normalized_scores[ci];

            // Max similarity to already selected items
            let max_sim = if selected.is_empty() {
                0.0
            } else {
                selected.iter()
                    .map(|&si| jaccard_similarity(&token_sets[ci], &token_sets[si]))
                    .fold(0.0_f32, f32::max)
            };

            let mmr = lambda * relevance - (1.0 - lambda) * max_sim;

            if mmr > best_mmr {
                best_mmr = mmr;
                best_idx_in_remaining = ri;
            }
        }

        let chosen = remaining.remove(best_idx_in_remaining);
        selected.push(chosen);
        result.push((candidates[chosen].0, candidates[chosen].1));
    }

    result
}
