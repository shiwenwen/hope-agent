//! Generic fuzzy-match helper used by `/model`, `/project`, and any future
//! slash command that needs to resolve a user-supplied name (or id) to one
//! item in a list.
//!
//! Search order:
//!
//! 1. Exact match against any of the candidate's keys (case-insensitive).
//! 2. Unique prefix match across keys.
//! 3. Unique substring match across keys.
//!
//! Returns an `Ambiguous` error listing matches when more than one item
//! qualifies at the prefix/substring stage.

/// Resolve `query` to exactly one item.
///
/// `keys` returns the searchable strings for an item — typically two entries
/// like `[name, id]` so the user can type either. They are lowercased before
/// comparison; pass them in their natural case.
///
/// `label` is what we show in error messages when several items match
/// ambiguously (typically the item's display name).
///
/// `kind` names the resource type for the "no match" error message
/// (e.g. `"model"`, `"project"`).
pub fn fuzzy_match_one<T, K, L>(
    items: &[T],
    query: &str,
    keys: K,
    label: L,
    kind: &str,
) -> Result<T, String>
where
    T: Clone,
    K: Fn(&T) -> Vec<String>,
    L: Fn(&T) -> String,
{
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Err(format!("Empty {} query", kind));
    }

    let lowered_keys: Vec<Vec<String>> = items
        .iter()
        .map(|item| keys(item).into_iter().map(|k| k.to_lowercase()).collect())
        .collect();

    if let Some(idx) = lowered_keys
        .iter()
        .position(|ks| ks.iter().any(|k| k == &q))
    {
        return Ok(items[idx].clone());
    }

    let prefix: Vec<usize> = lowered_keys
        .iter()
        .enumerate()
        .filter(|(_, ks)| ks.iter().any(|k| k.starts_with(&q)))
        .map(|(i, _)| i)
        .collect();
    if prefix.len() == 1 {
        return Ok(items[prefix[0]].clone());
    }

    let contains: Vec<usize> = lowered_keys
        .iter()
        .enumerate()
        .filter(|(_, ks)| ks.iter().any(|k| k.contains(&q)))
        .map(|(i, _)| i)
        .collect();
    if contains.len() == 1 {
        return Ok(items[contains[0]].clone());
    }

    if contains.is_empty() {
        Err(format!("No {} matching `{}`", kind, query))
    } else {
        let names: Vec<String> = contains
            .iter()
            .map(|&i| format!("`{}`", label(&items[i])))
            .collect();
        Err(format!(
            "Ambiguous {} `{}`. Matches: {}",
            kind,
            query,
            names.join(", ")
        ))
    }
}
