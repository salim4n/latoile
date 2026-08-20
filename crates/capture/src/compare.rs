//! Deterministic pixel, geometry and accessibility comparison. Classification
//! thresholds live in core; this module only computes immutable measurements.

use crate::CaptureError;
use crate::artifacts::BaselineArtifacts;
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder, Rgba, RgbaImage};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const PIXEL_CHANNEL_TOLERANCE: u8 = 8;
const MAX_AX_EXAMPLES: usize = 100;

pub(super) struct ComputedDiff {
    pub changed_pixels: u64,
    pub total_pixels: u64,
    pub max_geometry_delta_milli: u32,
    pub accessibility_changes: u32,
    pub pixel_diff_png: Vec<u8>,
    pub heatmap_png: Vec<u8>,
    pub geometry_diff: Vec<u8>,
    pub accessibility_diff: Vec<u8>,
}

pub(super) fn compare(
    baseline: &BaselineArtifacts,
    render_png: &[u8],
    render_geometry: &Value,
    render_accessibility: &Value,
) -> Result<ComputedDiff, CaptureError> {
    let baseline_png = image::load_from_memory(&baseline.png)
        .map_err(|error| CaptureError::Evidence(error.to_string()))?
        .to_rgba8();
    let render_png = image::load_from_memory(render_png)
        .map_err(|error| CaptureError::Evidence(error.to_string()))?
        .to_rgba8();
    if baseline_png.dimensions() != render_png.dimensions() {
        return Err(CaptureError::Evidence(
            "baseline and live render dimensions differ".into(),
        ));
    }
    let (changed_pixels, pixel_diff_png, heatmap_png) = compare_pixels(&baseline_png, &render_png)?;
    let baseline_geometry: Value = serde_json::from_slice(&baseline.geometry)
        .map_err(|error| CaptureError::Evidence(error.to_string()))?;
    let (max_geometry_delta_milli, geometry_diff) =
        compare_geometry(&baseline_geometry, render_geometry)?;
    let baseline_accessibility: Value = serde_json::from_slice(&baseline.accessibility)
        .map_err(|error| CaptureError::Evidence(error.to_string()))?;
    let (accessibility_changes, accessibility_diff) =
        compare_accessibility(&baseline_accessibility, render_accessibility)?;
    Ok(ComputedDiff {
        changed_pixels,
        total_pixels: u64::from(baseline_png.width()) * u64::from(baseline_png.height()),
        max_geometry_delta_milli,
        accessibility_changes,
        pixel_diff_png,
        heatmap_png,
        geometry_diff,
        accessibility_diff,
    })
}

fn compare_pixels(
    baseline: &RgbaImage,
    render: &RgbaImage,
) -> Result<(u64, Vec<u8>, Vec<u8>), CaptureError> {
    let mut changed = 0_u64;
    let mut pixel_diff = RgbaImage::new(baseline.width(), baseline.height());
    let mut heatmap = RgbaImage::new(baseline.width(), baseline.height());
    for (x, y, baseline_pixel) in baseline.enumerate_pixels() {
        let render_pixel = render.get_pixel(x, y);
        let deltas = [
            baseline_pixel[0].abs_diff(render_pixel[0]),
            baseline_pixel[1].abs_diff(render_pixel[1]),
            baseline_pixel[2].abs_diff(render_pixel[2]),
            baseline_pixel[3].abs_diff(render_pixel[3]),
        ];
        let is_changed = deltas.iter().copied().max().unwrap_or(0) > PIXEL_CHANNEL_TOLERANCE;
        if is_changed {
            changed += 1;
        }
        pixel_diff.put_pixel(
            x,
            y,
            Rgba([
                deltas[0].saturating_mul(4),
                deltas[1].saturating_mul(4),
                deltas[2].saturating_mul(4),
                255,
            ]),
        );
        let gray = ((u16::from(baseline_pixel[0])
            + u16::from(baseline_pixel[1])
            + u16::from(baseline_pixel[2]))
            / 3) as u8;
        heatmap.put_pixel(
            x,
            y,
            if is_changed {
                Rgba([255, 24, 72, 255])
            } else {
                Rgba([gray, gray, gray, 96])
            },
        );
    }
    Ok((changed, encode_png(&pixel_diff)?, encode_png(&heatmap)?))
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, CaptureError> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ColorType::Rgba8.into(),
        )
        .map_err(|error| CaptureError::Evidence(error.to_string()))?;
    Ok(bytes)
}

fn geometry_by_selector(value: &Value) -> Result<BTreeMap<String, Value>, CaptureError> {
    value
        .as_array()
        .ok_or_else(|| CaptureError::Evidence("geometry snapshot is not an array".into()))?
        .iter()
        .map(|entry| {
            let selector = entry
                .get("selector")
                .and_then(Value::as_str)
                .ok_or_else(|| CaptureError::Evidence("geometry selector is missing".into()))?;
            Ok((selector.to_string(), entry.clone()))
        })
        .collect()
}

fn compare_geometry(baseline: &Value, render: &Value) -> Result<(u32, Vec<u8>), CaptureError> {
    let baseline = geometry_by_selector(baseline)?;
    let render = geometry_by_selector(render)?;
    if baseline.keys().collect::<Vec<_>>() != render.keys().collect::<Vec<_>>() {
        return Err(CaptureError::Evidence(
            "live geometry selectors do not match the approved scenario".into(),
        ));
    }
    let mut max_delta_milli = 0_u32;
    let mut changes = Vec::new();
    for (selector, expected) in &baseline {
        let actual = &render[selector];
        let mut selector_delta = 0_u32;
        for field in ["x", "y", "width", "height"] {
            let expected_value = expected
                .pointer(&format!("/rect/{field}"))
                .and_then(Value::as_f64)
                .ok_or_else(|| CaptureError::Evidence("baseline rectangle is invalid".into()))?;
            let actual_value = actual
                .pointer(&format!("/rect/{field}"))
                .and_then(Value::as_f64)
                .ok_or_else(|| CaptureError::Evidence("render rectangle is invalid".into()))?;
            selector_delta = selector_delta.max(
                ((expected_value - actual_value).abs() * 1000.0)
                    .round()
                    .min(f64::from(u32::MAX)) as u32,
            );
        }
        if expected != actual {
            if selector_delta == 0 {
                selector_delta = 1;
            }
            changes.push(json!({
                "selector": selector,
                "max_delta_milli": selector_delta,
                "baseline": expected,
                "render": actual,
            }));
            max_delta_milli = max_delta_milli.max(selector_delta);
        }
    }
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "max_delta_milli": max_delta_milli,
        "changes": changes,
    }))
    .map_err(|error| CaptureError::Evidence(error.to_string()))?;
    Ok((max_delta_milli, bytes))
}

fn ax_multiset(value: &Value) -> Result<BTreeMap<String, u32>, CaptureError> {
    let nodes = value
        .as_array()
        .ok_or_else(|| CaptureError::Evidence("accessibility snapshot is not an array".into()))?;
    let mut set = BTreeMap::new();
    for node in nodes {
        let canonical = serde_json::to_string(node)
            .map_err(|error| CaptureError::Evidence(error.to_string()))?;
        *set.entry(canonical).or_insert(0) += 1;
    }
    Ok(set)
}

fn compare_accessibility(baseline: &Value, render: &Value) -> Result<(u32, Vec<u8>), CaptureError> {
    let baseline = ax_multiset(baseline)?;
    let render = ax_multiset(render)?;
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut changes = 0_u32;
    for (node, expected_count) in &baseline {
        let actual_count = render.get(node).copied().unwrap_or(0);
        if *expected_count > actual_count {
            let count = *expected_count - actual_count;
            changes = changes.saturating_add(count);
            if removed.len() < MAX_AX_EXAMPLES {
                removed.push(json!({
                    "count": count,
                    "node": serde_json::from_str::<Value>(node).unwrap_or(Value::Null),
                }));
            }
        }
    }
    for (node, actual_count) in &render {
        let expected_count = baseline.get(node).copied().unwrap_or(0);
        if *actual_count > expected_count {
            let count = *actual_count - expected_count;
            changes = changes.saturating_add(count);
            if added.len() < MAX_AX_EXAMPLES {
                added.push(json!({
                    "count": count,
                    "node": serde_json::from_str::<Value>(node).unwrap_or(Value::Null),
                }));
            }
        }
    }
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "change_count": changes,
        "examples_truncated": usize::try_from(changes).unwrap_or(usize::MAX) > MAX_AX_EXAMPLES,
        "removed": removed,
        "added": added,
    }))
    .map_err(|error| CaptureError::Evidence(error.to_string()))?;
    Ok((changes, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_with_box(offset: u32) -> Vec<u8> {
        let mut image = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        for y in 20..60 {
            for x in offset..offset + 40 {
                image.put_pixel(x, y, Rgba([24, 24, 24, 255]));
            }
        }
        encode_png(&image).unwrap()
    }

    #[test]
    fn a_known_spacing_mismatch_produces_blocking_measurements() {
        let baseline_geometry = json!([{
            "selector": "main",
            "rect": {"x": 20.0, "y": 20.0, "width": 40.0, "height": 40.0},
            "tag": "main", "role": "main", "text": "Card", "masked": false, "style": {}
        }]);
        let render_geometry = json!([{
            "selector": "main",
            "rect": {"x": 36.0, "y": 20.0, "width": 40.0, "height": 40.0},
            "tag": "main", "role": "main", "text": "Card", "masked": false, "style": {}
        }]);
        let baseline = BaselineArtifacts {
            png: png_with_box(20),
            geometry: serde_json::to_vec(&baseline_geometry).unwrap(),
            accessibility: b"[]".to_vec(),
            environment: b"{}".to_vec(),
        };
        let diff = compare(&baseline, &png_with_box(36), &render_geometry, &json!([])).unwrap();
        assert!(diff.changed_pixels > 1_000);
        assert_eq!(diff.max_geometry_delta_milli, 16_000);
        assert!(!diff.pixel_diff_png.is_empty());
        assert!(!diff.heatmap_png.is_empty());
    }
}
