use anyhow::Result;
use serde_json::Value;

const WEB_FETCH_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const DEFAULT_WEB_FETCH_MAX_CHARS: usize = 50000;
const WEB_FETCH_TIMEOUT_SECS: u64 = 30;

pub(crate) async fn tool_web_search(args: &Value) -> Result<String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(10) as usize;

    log::info!("Web search: {} (count: {})", query, count);

    let client = reqwest::Client::builder()
        .user_agent(WEB_FETCH_USER_AGENT)
        .timeout(std::time::Duration::from_secs(WEB_FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

    let search_url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );

    let resp = client
        .get(&search_url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Search request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "Search failed with status: {}",
            resp.status()
        ));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read search response: {}", e))?;

    let results = parse_ddg_results(&html, count);

    if results.is_empty() {
        return Ok(format!("No results found for: {}", query));
    }

    let mut output = format!("Search results for: {}\n\n", query);
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   {}\n\n",
            i + 1,
            result.title,
            result.url,
            result.snippet
        ));
    }

    Ok(output)
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

fn parse_ddg_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < max_results {
        let link_marker = "class=\"result__a\"";
        let link_start = match html[pos..].find(link_marker) {
            Some(idx) => pos + idx,
            None => break,
        };

        let href_start = match html[..link_start].rfind("href=\"") {
            Some(idx) => idx + 6,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let href_end = match html[href_start..].find('"') {
            Some(idx) => href_start + idx,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let raw_url = &html[href_start..href_end];
        let url = extract_ddg_url(raw_url);

        let title_start = match html[link_start..].find('>') {
            Some(idx) => link_start + idx + 1,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let title_end = match html[title_start..].find("</a>") {
            Some(idx) => title_start + idx,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let title = strip_html_tags(&html[title_start..title_end]);

        let snippet_marker = "class=\"result__snippet\"";
        let snippet = if let Some(snippet_start) = html[title_end..].find(snippet_marker) {
            let abs_snippet_start = title_end + snippet_start;
            if let Some(tag_end) = html[abs_snippet_start..].find('>') {
                let content_start = abs_snippet_start + tag_end + 1;
                if let Some(end) = html[content_start..].find("</a>") {
                    strip_html_tags(&html[content_start..content_start + end])
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: html_decode(&title),
                url,
                snippet: html_decode(&snippet),
            });
        }

        pos = title_end;
    }

    results
}

fn extract_ddg_url(raw: &str) -> String {
    if let Some(uddg_start) = raw.find("uddg=") {
        let url_start = uddg_start + 5;
        let url_end = raw[url_start..]
            .find('&')
            .map(|i| url_start + i)
            .unwrap_or(raw.len());
        let encoded = &raw[url_start..url_end];
        urlencoding::decode(encoded)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| encoded.to_string())
    } else if raw.starts_with("http") {
        raw.to_string()
    } else {
        raw.to_string()
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(c);
        }
    }
    result.trim().to_string()
}

fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

pub(crate) async fn tool_web_fetch(args: &Value) -> Result<String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_WEB_FETCH_MAX_CHARS as u64) as usize;

    log::info!("Fetching URL: {} (max_chars: {})", url, max_chars);

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(anyhow::anyhow!(
            "Invalid URL: must start with http:// or https://"
        ));
    }

    let client = reqwest::Client::builder()
        .user_agent(WEB_FETCH_USER_AGENT)
        .timeout(std::time::Duration::from_secs(WEB_FETCH_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get(url)
        .header("Accept", "text/html,application/json,text/plain,*/*")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Fetch request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("Fetch failed with status: {}", status));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body = resp
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;

    let text = if content_type.contains("text/html") {
        extract_readable_text(&body)
    } else if content_type.contains("application/json") {
        match serde_json::from_str::<Value>(&body) {
            Ok(v) => serde_json::to_string_pretty(&v).unwrap_or(body),
            Err(_) => body,
        }
    } else {
        body
    };

    if text.len() > max_chars {
        let truncated = &text[..max_chars];
        Ok(format!(
            "URL: {}\nContent-Type: {}\n\n{}\n\n... (content truncated, {} chars total)",
            url,
            content_type,
            truncated,
            text.len()
        ))
    } else {
        Ok(format!(
            "URL: {}\nContent-Type: {}\n\n{}",
            url, content_type, text
        ))
    }
}

fn extract_readable_text(html: &str) -> String {
    let mut result = String::with_capacity(html.len() / 2);
    let mut pos = 0;
    let lower = html.to_lowercase();

    let mut cleaned = String::with_capacity(html.len());
    while pos < html.len() {
        let remaining_lower = &lower[pos..];
        if remaining_lower.starts_with("<script") {
            if let Some(end) = lower[pos..].find("</script>") {
                pos += end + 9;
                continue;
            }
        }
        if remaining_lower.starts_with("<style") {
            if let Some(end) = lower[pos..].find("</style>") {
                pos += end + 8;
                continue;
            }
        }
        if remaining_lower.starts_with("<noscript") {
            if let Some(end) = lower[pos..].find("</noscript>") {
                pos += end + 11;
                continue;
            }
        }
        if remaining_lower.starts_with("<nav") {
            if let Some(end) = lower[pos..].find("</nav>") {
                pos += end + 6;
                continue;
            }
        }
        cleaned.push(html.as_bytes()[pos] as char);
        pos += 1;
    }

    let mut in_tag = false;
    let mut last_was_space = false;
    let mut newline_count = 0;

    for c in cleaned.chars() {
        if c == '<' {
            in_tag = true;
            continue;
        }
        if c == '>' {
            in_tag = false;
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
            continue;
        }
        if in_tag {
            continue;
        }
        if c == '\n' || c == '\r' {
            newline_count += 1;
            if newline_count <= 2 && !last_was_space {
                result.push('\n');
                last_was_space = true;
            }
            continue;
        }
        if c.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
            continue;
        }
        newline_count = 0;
        last_was_space = false;
        result.push(c);
    }

    html_decode(result.trim())
}
