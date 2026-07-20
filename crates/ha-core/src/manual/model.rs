//! Chapter parsing: embed keys → typed chapters with heading slugs.

use std::collections::HashMap;

use super::{ManualChapter, ManualHeading};

/// Parsed chapters for one manual language (`"zh"` / `"en"`), README index
/// (number 0) first, then chapters ascending. Release builds cache the parse
/// per language (the embed is fixed for the binary's lifetime); debug builds
/// re-parse per call so on-disk doc edits show up live.
pub(super) fn chapters(lang: &str) -> Vec<ManualChapter> {
    #[cfg(not(debug_assertions))]
    {
        static CACHE: std::sync::OnceLock<(Vec<ManualChapter>, Vec<ManualChapter>)> =
            std::sync::OnceLock::new();
        let (zh, en) = CACHE.get_or_init(|| (parse_chapters("zh"), parse_chapters("en")));
        if lang == "zh" {
            zh.clone()
        } else {
            en.clone()
        }
    }
    #[cfg(debug_assertions)]
    parse_chapters(lang)
}

fn parse_chapters(lang: &str) -> Vec<ManualChapter> {
    let mut out: Vec<ManualChapter> = Vec::new();
    for (key, data) in super::embed::manual_files() {
        // Language is decided by path prefix, chapter number by the ASCII
        // digit prefix of the basename — the (non-ASCII) rest of the filename
        // is deliberately never interpreted.
        let (file_lang, basename) = match key.strip_prefix("en/") {
            Some(rest) => ("en", rest),
            None => ("zh", key.as_str()),
        };
        if file_lang != lang || basename.contains('/') || !basename.ends_with(".md") {
            continue;
        }
        let number = chapter_number(basename);
        let Some(number) = number else { continue };
        let Ok(body) = String::from_utf8(data.into_owned()) else {
            crate::app_warn!("manual", "parse", "embedded manual file {key} is not UTF-8");
            continue;
        };
        let headings = parse_headings(&body);
        let title = chapter_title(&headings, basename);
        out.push(ManualChapter {
            number,
            title,
            body,
            headings,
        });
    }
    out.sort_by_key(|c| c.number);
    out
}

/// `README.md` → 0; `NN-….md` → NN. Anything else is not a chapter.
pub(super) fn chapter_number(basename: &str) -> Option<u8> {
    if basename.eq_ignore_ascii_case("README.md") {
        return Some(0);
    }
    let digits: String = basename
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.len() == 2 {
        digits.parse().ok()
    } else {
        None
    }
}

/// Title from the first H1, with a `NN · ` / `NN - ` prefix stripped.
fn chapter_title(headings: &[ManualHeading], basename: &str) -> String {
    let h1 = headings.iter().find(|h| h.level == 1);
    let Some(h1) = h1 else {
        return basename.trim_end_matches(".md").to_string();
    };
    let text = h1.text.trim();
    // `# 04 · 记忆系统` / `# 04 · Memory` → keep only the title part.
    let stripped = text
        .strip_prefix(|c: char| c.is_ascii_digit())
        .and_then(|rest| rest.strip_prefix(|c: char| c.is_ascii_digit()))
        .map(|rest| rest.trim_start_matches([' ', '·', '-', '—']).trim());
    match stripped {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => text.to_string(),
    }
}

/// Fence-aware ATX heading scan (ports the frontend `outline.ts` logic) with
/// GitHub-style slug assignment including duplicate `-N` suffixes.
pub(super) fn parse_headings(md: &str) -> Vec<ManualHeading> {
    let mut out = Vec::new();
    let mut seen: HashMap<String, u32> = HashMap::new();
    // (fence char, fence length) while inside a fenced code block.
    let mut fence: Option<(char, usize)> = None;
    for (idx, raw) in md.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed_start = line.trim_start_matches(' ');
        let indent = line.len() - trimmed_start.len();
        if indent <= 3 {
            let marker_char = trimmed_start.chars().next();
            if matches!(marker_char, Some('`') | Some('~')) {
                let ch = marker_char.unwrap();
                let len = trimmed_start.chars().take_while(|&c| c == ch).count();
                if len >= 3 {
                    match fence {
                        None => {
                            fence = Some((ch, len));
                            continue;
                        }
                        Some((open_ch, open_len)) => {
                            // A closing fence: same char, at least as long,
                            // no trailing info string.
                            if ch == open_ch
                                && len >= open_len
                                && trimmed_start[len..].trim().is_empty()
                            {
                                fence = None;
                            }
                            continue;
                        }
                    }
                }
            }
        }
        if fence.is_some() || indent > 3 {
            continue;
        }
        let hashes = trimmed_start.chars().take_while(|&c| c == '#').count();
        if hashes == 0 || hashes > 6 {
            continue;
        }
        let rest = &trimmed_start[hashes..];
        if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
            continue;
        }
        // Strip trailing closing hashes (`## title ##`).
        let mut text = rest.trim().to_string();
        let closing = text.trim_end_matches('#');
        if closing.len() < text.len() && closing.ends_with([' ', '\t']) {
            text = closing.trim_end().to_string();
        }
        if text.is_empty() {
            continue;
        }
        let base = github_slug(&text);
        let n = seen.entry(base.clone()).or_insert(0);
        let slug = if *n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        *n += 1;
        out.push(ManualHeading {
            level: hashes as u8,
            text,
            slug,
            line: (idx + 1) as u32,
        });
    }
    out
}

/// GitHub-style anchor slug: lowercase, drop punctuation/symbols (CJK and
/// other Unicode alphanumerics survive), spaces → `-` without collapsing
/// runs. Must stay byte-identical to the frontend `manualSlug.ts`.
pub(super) fn github_slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if c == ' ' {
            out.push('-');
        } else if c == '-' || c == '_' {
            out.push(c);
        } else if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        }
        // Everything else (punctuation, symbols, emoji) is dropped.
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_match_github_anchors_from_the_real_docs() {
        // Ground-truth pairs taken from anchors that exist in docs/user-guide.
        for (heading, anchor) in [
            (
                "4.1 三层记忆：全局 / Agent / 项目",
                "41-三层记忆全局--agent--项目",
            ),
            ("7.8 电脑控制（macOS）", "78-电脑控制macos"),
            ("2.11 语音转写(STT)", "211-语音转写stt"),
            (
                "2.4 Sign in with a ChatGPT / Codex account",
                "24-sign-in-with-a-chatgpt--codex-account",
            ),
            ("13.1 设置界面导航地图", "131-设置界面导航地图"),
            (
                "Core concepts (all in one place)",
                "core-concepts-all-in-one-place",
            ),
        ] {
            assert_eq!(github_slug(heading), anchor, "heading: {heading}");
        }
    }

    #[test]
    fn duplicate_headings_get_numeric_suffixes() {
        let md = "# T\n## Same\n## Same\n### Same\n";
        let hs = parse_headings(md);
        let slugs: Vec<_> = hs.iter().map(|h| h.slug.as_str()).collect();
        assert_eq!(slugs, ["t", "same", "same-1", "same-2"]);
    }

    #[test]
    fn fenced_code_headings_are_skipped() {
        let md = "# Real\n```bash\n# comment not a heading\n```\n## After\n";
        let hs = parse_headings(md);
        let texts: Vec<_> = hs.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(texts, ["Real", "After"]);
    }

    #[test]
    fn parses_all_chapters_for_both_languages() {
        for lang in ["zh", "en"] {
            let chapters = parse_chapters(lang);
            let numbers: Vec<u8> = chapters.iter().map(|c| c.number).collect();
            // Contiguous 0..=max without pinning the max — new chapters land
            // without touching this test; gaps or duplicates still fail.
            let max = *numbers.last().unwrap_or(&0);
            assert!(max >= 13, "{lang} manual lost chapters: {numbers:?}");
            assert_eq!(
                numbers,
                (0..=max).collect::<Vec<u8>>(),
                "{lang} chapter numbers not contiguous: {numbers:?}"
            );
            for c in &chapters {
                assert!(
                    !c.title.is_empty(),
                    "{lang} chapter {} has no title",
                    c.number
                );
                assert!(
                    !c.headings.is_empty(),
                    "{lang} chapter {} has no headings",
                    c.number
                );
            }
        }
    }

    /// Contract test against the real corpus: every intra-doc `#anchor` link
    /// (same-chapter or cross-chapter `NN-….md#anchor`) must resolve to a
    /// computed heading slug. Catches slug-algorithm drift AND doc typos.
    #[test]
    fn every_intra_doc_anchor_resolves_to_a_computed_slug() {
        let link_re = regex::Regex::new(r"\]\(([^)\s]+)\)").unwrap();
        for lang in ["zh", "en"] {
            let chapters = parse_chapters(lang);
            let slug_sets: std::collections::HashMap<u8, std::collections::HashSet<&str>> =
                chapters
                    .iter()
                    .map(|c| {
                        (
                            c.number,
                            c.headings.iter().map(|h| h.slug.as_str()).collect(),
                        )
                    })
                    .collect();
            for c in &chapters {
                for cap in link_re.captures_iter(&c.body) {
                    let target = &cap[1];
                    let (file_part, anchor) = match target.split_once('#') {
                        Some((f, a)) => (f, a),
                        None => continue,
                    };
                    if anchor.is_empty() || target.starts_with("http") {
                        continue;
                    }
                    let dest_chapter = if file_part.is_empty() {
                        Some(c.number)
                    } else {
                        // Only same-language chapter files; links that walk
                        // out of the manual are handled by the frontend.
                        let base = file_part.rsplit('/').next().unwrap_or(file_part);
                        if file_part.contains("..") {
                            None
                        } else {
                            chapter_number(base)
                        }
                    };
                    let Some(dest) = dest_chapter else { continue };
                    let slugs = slug_sets.get(&dest).unwrap_or_else(|| {
                        panic!("{lang} ch{} links to missing chapter {dest}", c.number)
                    });
                    assert!(
                        slugs.contains(anchor),
                        "{lang} chapter {} link `{}` → anchor `{}` not found in chapter {}",
                        c.number,
                        target,
                        anchor,
                        dest
                    );
                }
            }
        }
    }
}
