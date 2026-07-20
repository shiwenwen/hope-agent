//! Line-granular full-text search over the parsed manual.
//!
//! No FTS index and no tokenizer: the corpus is ~300 KB, a linear scan is
//! sub-millisecond. The query is split on whitespace and every term must
//! match as a case-insensitive Unicode substring of the line (AND). This
//! covers English (`"memory recall"` → two word terms), Chinese
//! (`"记忆召回"` → one term, substring match needs no word boundaries), and
//! mixed queries uniformly.

use super::{ManualChapter, ManualSearchHit};

/// STX/ETX hit markers — the contract of the frontend
/// `renderHighlightedSnippet` helper (mirrors the session-search backend).
const HIT_START: char = '\u{2}';
const HIT_END: char = '\u{3}';

/// Max results returned; the UI shows a flat list, deep tails are noise.
const MAX_HITS: usize = 50;
/// Snippet window (in chars) for long lines, centered on the first hit.
const SNIPPET_WINDOW: usize = 160;

pub(super) fn search_chapters(chapters: &[ManualChapter], query: &str) -> Vec<ManualSearchHit> {
    let terms: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|t| lower_chars(t))
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }

    let mut hits: Vec<ManualSearchHit> = Vec::new();
    for chapter in chapters {
        for (idx, raw) in chapter.body.split('\n').enumerate() {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.trim().is_empty() {
                continue;
            }
            let chars: Vec<char> = line.chars().collect();
            let lower = lower_chars(line);
            // All terms must appear (AND); collect every occurrence range.
            let mut ranges: Vec<(usize, usize)> = Vec::new();
            let mut occurrences = 0usize;
            let mut all_matched = true;
            for term in &terms {
                let starts = find_all(&lower, term);
                if starts.is_empty() {
                    all_matched = false;
                    break;
                }
                occurrences += starts.len();
                ranges.extend(starts.iter().map(|&s| (s, s + term.len())));
            }
            if !all_matched {
                continue;
            }
            let line_no = (idx + 1) as u32;
            let is_heading = chapter.headings.iter().any(|h| h.line == line_no);
            let anchor = chapter
                .headings
                .iter()
                .rev()
                .find(|h| h.line <= line_no && h.level > 1)
                .map(|h| h.slug.clone());
            let mut score = (occurrences as i32) * 10;
            if is_heading {
                score += 100;
            }
            // Slight bias toward earlier chapters on ties.
            score += i32::from(14u8.saturating_sub(chapter.number));
            hits.push(ManualSearchHit {
                chapter: chapter.number,
                chapter_title: chapter.title.clone(),
                anchor,
                line: line_no,
                snippet: build_snippet(&chars, &ranges),
                score,
            });
        }
    }
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.chapter.cmp(&b.chapter))
            .then(a.line.cmp(&b.line))
    });
    hits.truncate(MAX_HITS);
    hits
}

/// Per-char lowercase keeping a 1:1 index mapping with the original chars
/// (multi-char expansions like `ß`→`ss` are truncated — fine for search).
fn lower_chars(s: &str) -> Vec<char> {
    s.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

/// All start indices of `needle` in `haystack` (non-overlapping).
fn find_all(haystack: &[char], needle: &[char]) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || haystack.len() < needle.len() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if haystack[i..i + needle.len()] == needle[..] {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Window the line around the first hit and wrap every hit range in
/// STX/ETX. `ranges` are char offsets into `chars`.
fn build_snippet(chars: &[char], ranges: &[(usize, usize)]) -> String {
    let mut merged = merge_ranges(ranges);
    let (start, end) = if chars.len() <= SNIPPET_WINDOW {
        (0, chars.len())
    } else {
        let first_hit = merged.first().map(|&(s, _)| s).unwrap_or(0);
        let half = SNIPPET_WINDOW / 2;
        let start = first_hit.saturating_sub(half);
        let end = (start + SNIPPET_WINDOW).min(chars.len());
        (end.saturating_sub(SNIPPET_WINDOW), end)
    };
    merged.retain(|&(s, _)| s >= start && s < end);
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    let mut cursor = start;
    for &(s, e) in merged.iter() {
        let e = e.min(end);
        out.extend(&chars[cursor..s]);
        out.push(HIT_START);
        out.extend(&chars[s..e]);
        out.push(HIT_END);
        cursor = e;
    }
    out.extend(&chars[cursor..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// Sort + merge overlapping char ranges.
fn merge_ranges(ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut sorted: Vec<(usize, usize)> = ranges.to_vec();
    sorted.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in sorted {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::super::model;
    use super::*;

    fn zh_chapters() -> Vec<ManualChapter> {
        let all = model::chapters("zh");
        assert!(!all.is_empty());
        all
    }

    #[test]
    fn chinese_query_hits_without_word_boundaries() {
        let hits = search_chapters(&zh_chapters(), "知识空间");
        assert!(!hits.is_empty());
        // Chapter 5 is the Knowledge Space chapter; a heading hit there must
        // rank near the top.
        assert!(
            hits.iter().take(5).any(|h| h.chapter == 5),
            "top hits: {:?}",
            hits.iter()
                .take(5)
                .map(|h| (h.chapter, h.line, h.score))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn mixed_language_terms_are_anded() {
        let hits = search_chapters(&zh_chapters(), "Obsidian 绑定");
        assert!(!hits.is_empty());
        for h in &hits {
            let plain: String = h
                .snippet
                .chars()
                .filter(|&c| c != HIT_START && c != HIT_END)
                .collect();
            let lower = plain.to_lowercase();
            assert!(
                lower.contains("obsidian") && lower.contains("绑定"),
                "{plain}"
            );
        }
    }

    #[test]
    fn snippet_marks_hits_with_stx_etx() {
        let hits = search_chapters(&zh_chapters(), "记忆");
        let first = hits.first().expect("some hit");
        assert!(first.snippet.contains(HIT_START) && first.snippet.contains(HIT_END));
    }

    #[test]
    fn no_terms_no_hits() {
        assert!(search_chapters(&zh_chapters(), "   ").is_empty());
        assert!(search_chapters(&zh_chapters(), "词绝不出现xyzzy").is_empty());
    }

    #[test]
    fn long_lines_are_windowed_around_the_hit() {
        let chapter = ManualChapter {
            number: 1,
            title: "T".into(),
            body: format!("{}目标词{}", "填".repeat(300), "充".repeat(300)),
            headings: Vec::new(),
        };
        let hits = search_chapters(&[chapter], "目标词");
        let snippet = &hits[0].snippet;
        assert!(snippet.chars().count() < 200, "window not applied");
        assert!(snippet.starts_with('…') && snippet.ends_with('…'));
        assert!(snippet.contains(HIT_START));
    }
}
