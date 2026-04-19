use anyhow::Result;
use serde_json::Value;

use super::{get_bool, get_str, get_u32, require_browser};
use crate::browser_state::get_browser_state;

pub(super) async fn action_click(args: &Value) -> Result<String> {
    require_browser().await?;
    let ref_id = get_u32(args, "ref")
        .ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter (element ref ID from snapshot)"))?;
    let double_click = get_bool(args, "double_click").unwrap_or(false);

    let state = get_browser_state().lock().await;
    let element_info = state.find_ref(ref_id)?.clone();
    let page = state.get_active_page()?;

    let el = page
        .find_element(&element_info.selector)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Element ref={} (selector: {}) not found on page: {}. Take a new snapshot.",
                ref_id,
                element_info.selector,
                e
            )
        })?;

    el.scroll_into_view().await.ok();
    el.click()
        .await
        .map_err(|e| anyhow::anyhow!("Click failed: {}", e))?;

    if double_click {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        el.click().await.ok();
    }

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    Ok(format!(
        "Clicked{} [ref={}] {} \"{}\"",
        if double_click { " (double)" } else { "" },
        ref_id,
        element_info.role,
        element_info.text
    ))
}

pub(super) async fn action_fill(args: &Value) -> Result<String> {
    require_browser().await?;
    let ref_id = get_u32(args, "ref").ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter"))?;
    let value =
        get_str(args, "value").ok_or_else(|| anyhow::anyhow!("Missing 'value' parameter"))?;

    let state = get_browser_state().lock().await;
    let element_info = state.find_ref(ref_id)?.clone();
    let page = state.get_active_page()?;

    let el = page
        .find_element(&element_info.selector)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Element ref={} not found: {}. Take a new snapshot.",
                ref_id,
                e
            )
        })?;

    el.scroll_into_view().await.ok();

    // Click to focus, clear existing content, then type new value
    el.click().await.ok();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Clear existing content via JS
    let clear_js = format!(
        "(() => {{ const el = document.querySelector('{}'); if (el) {{ el.value = ''; el.dispatchEvent(new Event('input', {{bubbles: true}})); }} }})()",
        element_info.selector.replace('\'', "\\'")
    );
    page.evaluate(clear_js).await.ok();

    // Type the new value
    el.type_str(value)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to type text: {}", e))?;

    Ok(format!(
        "Filled [ref={}] {} with \"{}\"",
        ref_id, element_info.role, value
    ))
}

pub(super) async fn action_fill_form(args: &Value) -> Result<String> {
    require_browser().await?;
    let fields = args
        .get("fields")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            anyhow::anyhow!("Missing 'fields' parameter (object mapping ref IDs to values)")
        })?;

    let mut results = Vec::new();

    for (ref_key, value) in fields {
        let ref_id: u32 = ref_key
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid ref ID: '{}'. Must be a number.", ref_key))?;
        let val = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Value for ref {} must be a string", ref_id))?;

        let sub_args = serde_json::json!({
            "ref": ref_id,
            "value": val
        });

        match action_fill(&sub_args).await {
            Ok(msg) => results.push(msg),
            Err(e) => results.push(format!("Error filling ref={}: {}", ref_id, e)),
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    Ok(results.join("\n"))
}

pub(super) async fn action_hover(args: &Value) -> Result<String> {
    require_browser().await?;
    let ref_id = get_u32(args, "ref").ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter"))?;

    let state = get_browser_state().lock().await;
    let element_info = state.find_ref(ref_id)?.clone();
    let page = state.get_active_page()?;

    let el = page
        .find_element(&element_info.selector)
        .await
        .map_err(|e| anyhow::anyhow!("Element ref={} not found: {}", ref_id, e))?;

    el.scroll_into_view().await.ok();

    // Get center point and dispatch mouse move
    let point = el
        .clickable_point()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot get element position: {}", e))?;

    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchMouseEventParams, DispatchMouseEventType,
    };

    page.execute(DispatchMouseEventParams::new(
        DispatchMouseEventType::MouseMoved,
        point.x,
        point.y,
    ))
    .await
    .map_err(|e| anyhow::anyhow!("Hover failed: {}", e))?;

    Ok(format!(
        "Hovered [ref={}] {} \"{}\"",
        ref_id, element_info.role, element_info.text
    ))
}

pub(super) async fn action_drag(args: &Value) -> Result<String> {
    require_browser().await?;
    let from_ref = get_u32(args, "ref")
        .ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter (source element)"))?;
    let to_ref = get_u32(args, "target_ref")
        .ok_or_else(|| anyhow::anyhow!("Missing 'target_ref' parameter (destination element)"))?;

    let state = get_browser_state().lock().await;
    let from_el = state.find_ref(from_ref)?.clone();
    let to_el = state.find_ref(to_ref)?.clone();
    let page = state.get_active_page()?;

    let from_elem = page
        .find_element(&from_el.selector)
        .await
        .map_err(|e| anyhow::anyhow!("Source element ref={} not found: {}", from_ref, e))?;
    let to_elem = page
        .find_element(&to_el.selector)
        .await
        .map_err(|e| anyhow::anyhow!("Target element ref={} not found: {}", to_ref, e))?;

    let from_point = from_elem.clickable_point().await?;
    let to_point = to_elem.clickable_point().await?;

    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
    };

    // Mouse down at source
    let mut down = DispatchMouseEventParams::new(
        DispatchMouseEventType::MousePressed,
        from_point.x,
        from_point.y,
    );
    down.button = Some(MouseButton::Left);
    down.click_count = Some(1);
    page.execute(down).await?;

    // Move to destination
    let mut mv =
        DispatchMouseEventParams::new(DispatchMouseEventType::MouseMoved, to_point.x, to_point.y);
    mv.button = Some(MouseButton::Left);
    page.execute(mv).await?;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Mouse up at destination
    let mut up = DispatchMouseEventParams::new(
        DispatchMouseEventType::MouseReleased,
        to_point.x,
        to_point.y,
    );
    up.button = Some(MouseButton::Left);
    up.click_count = Some(1);
    page.execute(up).await?;

    Ok(format!(
        "Dragged [ref={}] \"{}\" -> [ref={}] \"{}\"",
        from_ref, from_el.text, to_ref, to_el.text
    ))
}

pub(super) async fn action_press_key(args: &Value) -> Result<String> {
    require_browser().await?;
    let key = get_str(args, "key").ok_or_else(|| {
        anyhow::anyhow!("Missing 'key' parameter (e.g. 'Enter', 'Tab', 'Escape', 'a')")
    })?;

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchKeyEventParams, DispatchKeyEventType,
    };

    let mut down = DispatchKeyEventParams::new(DispatchKeyEventType::KeyDown);
    down.key = Some(key.to_string());
    page.execute(down)
        .await
        .map_err(|e| anyhow::anyhow!("Key press failed: {}", e))?;

    let mut up = DispatchKeyEventParams::new(DispatchKeyEventType::KeyUp);
    up.key = Some(key.to_string());
    page.execute(up).await.ok();

    Ok(format!("Pressed key: {}", key))
}

pub(super) async fn action_upload_file(args: &Value) -> Result<String> {
    require_browser().await?;
    let ref_id = get_u32(args, "ref").ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter"))?;
    let file_path = get_str(args, "file_path")
        .ok_or_else(|| anyhow::anyhow!("Missing 'file_path' parameter"))?;

    if !std::path::Path::new(file_path).exists() {
        return Err(anyhow::anyhow!("File not found: {}", file_path));
    }

    let state = get_browser_state().lock().await;
    let element_info = state.find_ref(ref_id)?.clone();
    let page = state.get_active_page()?;

    // Get the DOM node and set file via CDP
    use chromiumoxide::cdp::browser_protocol::dom::{
        GetDocumentParams, QuerySelectorParams, SetFileInputFilesParams,
    };

    let doc = page
        .execute(GetDocumentParams::default())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get document: {}", e))?;

    let node_id = doc.result.root.node_id;

    let query_result = page
        .execute(QuerySelectorParams::new(node_id, &element_info.selector))
        .await
        .map_err(|e| anyhow::anyhow!("Element ref={} not found for file upload: {}", ref_id, e))?;

    let file_node_id = query_result.result.node_id;

    let mut set_files = SetFileInputFilesParams::new(vec![file_path.to_string()]);
    set_files.node_id = Some(file_node_id);
    page.execute(set_files)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set file: {}", e))?;

    Ok(format!("Uploaded file '{}' to [ref={}]", file_path, ref_id))
}
