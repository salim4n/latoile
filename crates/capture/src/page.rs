//! Deterministic page preparation and evidence capture over CDP.

use crate::CaptureError;
use crate::cdp::CdpClient;
use base64::Engine;
use latoile_core::VisualBaselineCaptureRequest;
use serde_json::{Value, json};
use std::time::Duration;

const MAX_HTML_BYTES: usize = 10 * 1024 * 1024;
pub(super) const MAX_PNG_BYTES: usize = 50 * 1024 * 1024;

pub(super) fn validate_request(request: &VisualBaselineCaptureRequest) -> Result<(), CaptureError> {
    let lower = request.html.to_ascii_lowercase();
    if request.html.is_empty()
        || request.html.len() > MAX_HTML_BYTES
        || !lower.contains("<html")
        || lower.contains("<script")
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("file://")
        || lower.contains("ws://")
        || lower.contains("wss://")
        || lower.contains("<iframe")
        || lower.contains("<form")
    {
        return Err(CaptureError::UnsafeInput);
    }
    if request.scenario.stable_selectors.is_empty()
        || request.scenario.readiness_selector.trim().is_empty()
        || request
            .scenario
            .allowed_masks
            .iter()
            .any(|mask| !request.scenario.stable_selectors.contains(mask))
    {
        return Err(CaptureError::UnsafeInput);
    }
    Ok(())
}

pub(super) async fn configure_page(
    cdp: &mut CdpClient,
    request: &VisualBaselineCaptureRequest,
) -> Result<(), CaptureError> {
    cdp.call("Page.enable", json!({})).await?;
    cdp.call("Runtime.enable", json!({})).await?;
    cdp.call("Accessibility.enable", json!({})).await?;
    cdp.call("Network.enable", json!({})).await?;
    cdp.call(
        "Network.setBlockedURLs",
        json!({"urls": ["http://*", "https://*", "file://*", "ftp://*", "ws://*", "wss://*"]}),
    )
    .await?;
    cdp.call(
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": request.scenario.viewport_width,
            "height": request.scenario.viewport_height,
            "deviceScaleFactor": f64::from(request.scenario.device_scale_factor_milli) / 1000.0,
            "mobile": request.scenario.viewport_width <= 600,
            "screenWidth": request.scenario.viewport_width,
            "screenHeight": request.scenario.viewport_height,
        }),
    )
    .await?;
    cdp.call(
        "Emulation.setLocaleOverride",
        json!({"locale": request.scenario.locale}),
    )
    .await?;
    cdp.call(
        "Emulation.setEmulatedMedia",
        json!({
            "media": "screen",
            "features": [{"name": "prefers-color-scheme", "value": request.scenario.theme}],
        }),
    )
    .await?;
    let frame_tree = cdp.call("Page.getFrameTree", json!({})).await?;
    let frame_id = frame_tree
        .pointer("/frameTree/frame/id")
        .and_then(Value::as_str)
        .ok_or_else(|| CaptureError::Protocol("page frame id is missing".into()))?;
    cdp.call(
        "Page.setDocumentContent",
        json!({"frameId": frame_id, "html": request.html}),
    )
    .await?;
    evaluate(
        cdp,
        r#"(() => {
          const style = document.createElement('style');
          style.setAttribute('data-latoile-capture', 'motion-off');
          style.textContent = '*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important;scroll-behavior:auto!important}';
          document.head.appendChild(style);
          return document.fonts.ready.then(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true)))));
        })()"#,
        true,
    )
    .await?;
    Ok(())
}

pub(super) async fn wait_until_ready(
    cdp: &mut CdpClient,
    request: &VisualBaselineCaptureRequest,
) -> Result<(), CaptureError> {
    let selector = serde_json::to_string(&request.scenario.readiness_selector)
        .map_err(|error| CaptureError::Protocol(error.to_string()))?;
    let expression = format!(
        "(() => {{ try {{ return document.querySelectorAll({selector}).length === 1 && document.fonts.status === 'loaded'; }} catch (_) {{ return false; }} }})()"
    );
    for _ in 0..100 {
        if evaluate(cdp, &expression, false).await?.as_bool() == Some(true) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(CaptureError::ReadinessTimeout)
}

pub(super) async fn capture_geometry(
    cdp: &mut CdpClient,
    request: &VisualBaselineCaptureRequest,
) -> Result<Value, CaptureError> {
    let selectors = serde_json::to_string(&request.scenario.stable_selectors)
        .map_err(|error| CaptureError::Protocol(error.to_string()))?;
    let expression = format!(
        r#"(() => {{
          const round = value => Math.round(value * 1000) / 1000;
          return {selectors}.map(selector => {{
            const nodes = document.querySelectorAll(selector);
            if (nodes.length !== 1) throw new Error('unstable-selector');
            const node = nodes[0];
            const rect = node.getBoundingClientRect();
            const style = getComputedStyle(node);
            return {{
              selector,
              tag: node.tagName.toLowerCase(),
              role: node.getAttribute('role') || '',
              text: (node.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 200),
              rect: {{x: round(rect.x), y: round(rect.y), width: round(rect.width), height: round(rect.height)}},
              style: {{display: style.display, position: style.position, fontFamily: style.fontFamily, fontSize: style.fontSize, fontWeight: style.fontWeight, lineHeight: style.lineHeight}}
            }};
          }});
        }})()"#
    );
    evaluate(cdp, &expression, false)
        .await
        .map_err(|_| CaptureError::UnstableSelector)
}

pub(super) async fn capture_accessibility(cdp: &mut CdpClient) -> Result<Value, CaptureError> {
    let raw = cdp
        .call("Accessibility.getFullAXTree", json!({"depth": -1}))
        .await?;
    let nodes = raw
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| CaptureError::Protocol("accessibility tree is missing".into()))?;
    let canonical = nodes
        .iter()
        .map(|node| {
            let mut properties = node
                .get("properties")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|property| {
                    Some(json!({
                        "name": property.get("name")?.as_str()?,
                        "value": property.pointer("/value/value").cloned().unwrap_or(Value::Null),
                    }))
                })
                .collect::<Vec<_>>();
            properties.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
            json!({
                "ignored": node.get("ignored").and_then(Value::as_bool).unwrap_or(false),
                "role": node.pointer("/role/value").and_then(Value::as_str).unwrap_or(""),
                "name": node.pointer("/name/value").and_then(Value::as_str).unwrap_or(""),
                "description": node.pointer("/description/value").and_then(Value::as_str).unwrap_or(""),
                "properties": properties,
            })
        })
        .collect::<Vec<_>>();
    Ok(Value::Array(canonical))
}

pub(super) async fn capture_font_probe(
    cdp: &mut CdpClient,
    request: &VisualBaselineCaptureRequest,
) -> Result<Value, CaptureError> {
    let selectors = serde_json::to_string(&request.scenario.stable_selectors)
        .map_err(|error| CaptureError::Protocol(error.to_string()))?;
    let expression = format!(
        r#"(() => {{
          const canvas = document.createElement('canvas');
          const context = canvas.getContext('2d');
          const bodyFont = getComputedStyle(document.body).fontFamily;
          context.font = '16px ' + bodyFont;
          return {{
            bodyFont,
            glyphWidth: Math.round(context.measureText('LaToile fi 0123456789').width * 1000) / 1000,
            selectorFonts: {selectors}.map(selector => getComputedStyle(document.querySelector(selector)).fontFamily)
          }};
        }})()"#
    );
    evaluate(cdp, &expression, false)
        .await
        .map_err(|_| CaptureError::UnstableSelector)
}

pub(super) async fn apply_allowed_masks(
    cdp: &mut CdpClient,
    request: &VisualBaselineCaptureRequest,
) -> Result<(), CaptureError> {
    let masks = serde_json::to_string(&request.scenario.allowed_masks)
        .map_err(|error| CaptureError::Protocol(error.to_string()))?;
    let expression = format!(
        r#"(() => {{ {masks}.forEach(selector => {{
          const nodes = document.querySelectorAll(selector);
          if (nodes.length !== 1) throw new Error('unstable-mask');
          const node = nodes[0];
          node.style.setProperty('visibility', 'hidden', 'important');
        }}); return true; }})()"#
    );
    evaluate(cdp, &expression, false)
        .await
        .map(|_| ())
        .map_err(|_| CaptureError::UnstableSelector)
}

pub(super) async fn capture_png(
    cdp: &mut CdpClient,
    request: &VisualBaselineCaptureRequest,
) -> Result<Vec<u8>, CaptureError> {
    let result = cdp
        .call(
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "fromSurface": true,
                "captureBeyondViewport": false,
                "optimizeForSpeed": false,
            }),
        )
        .await?;
    let data = result
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| CaptureError::Protocol("screenshot bytes are missing".into()))?;
    let png = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| CaptureError::Protocol(error.to_string()))?;
    let expected_width = request
        .scenario
        .viewport_width
        .saturating_mul(request.scenario.device_scale_factor_milli)
        / 1000;
    let expected_height = request
        .scenario
        .viewport_height
        .saturating_mul(request.scenario.device_scale_factor_milli)
        / 1000;
    if png.len() > MAX_PNG_BYTES || png_dimensions(&png) != Some((expected_width, expected_height))
    {
        return Err(CaptureError::Protocol(
            "screenshot dimensions do not match the declared viewport".into(),
        ));
    }
    Ok(png)
}

async fn evaluate(
    cdp: &mut CdpClient,
    expression: &str,
    await_promise: bool,
) -> Result<Value, CaptureError> {
    let result = cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": await_promise,
                "returnByValue": true,
                "userGesture": false,
            }),
        )
        .await?;
    if result.get("exceptionDetails").is_some() {
        return Err(CaptureError::Protocol("page evaluation failed".into()));
    }
    Ok(result
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(png[16..20].try_into().ok()?),
        u32::from_be_bytes(png[20..24].try_into().ok()?),
    ))
}
