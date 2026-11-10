use super::{Encoding, Scan, TokenKind, config};
use std::ops::Range;

pub(super) fn scan(scan: &mut Scan<'_>) {
    let kind = scan.document.config.as_deref().unwrap_or_default();
    if !matches!(kind, "model" | "jm2" | "screen" | "texture" | "floorcol") {
        return;
    }
    let mut bytes = scan.document.text.as_bytes().to_vec();
    for token in config::tokens(scan.document, scan.index.config_types()) {
        if token.kind == TokenKind::Comment {
            for byte in &mut bytes[token.range] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
        }
    }
    let text = String::from_utf8(bytes).unwrap();
    if kind == "texture" {
        let mut start = 0;
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            if line.trim().starts_with('[') {
                texture(scan, &text, &text[start..offset]);
                start = offset;
            }
            offset += line.len();
        }
        texture(scan, &text, &text[start..]);
        return;
    }
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        if let Some(header) = line
            .strip_prefix("====")
            .and_then(|header| header.strip_suffix("===="))
        {
            section = header.trim();
            continue;
        }
        if kind == "jm2" && section == "LIGHTS" {
            if let Some((_, values)) = line.split_once(':') {
                for field in values.split_ascii_whitespace() {
                    if let Some(value) = field.strip_prefix("hsl=") {
                        scan.value(range(&text, value), Encoding::Hsl);
                    }
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let encoding = match kind {
            "model"
                if key
                    .strip_prefix("face")
                    .and_then(|key| key.strip_suffix(".colour"))
                    .is_some_and(|index| {
                        !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
                    }) =>
            {
                Some(Encoding::Hsl)
            }
            "jm2" if section == "ENV" && matches!(key, "sun_color" | "fog_color") => {
                Some(Encoding::Rgb)
            }
            "screen"
                if matches!(
                    key,
                    "colour"
                        | "text_colour"
                        | "fill_colour"
                        | "border_colour"
                        | "shadow_colour"
                        | "shadow"
                ) =>
            {
                Some(Encoding::Argb)
            }
            "floorcol" => {
                if let Some((underlay, overlay)) = value.split_once(',') {
                    scan.hex(range(&text, underlay.trim()), Encoding::Hsl32);
                    scan.hex(range(&text, overlay.trim()), Encoding::Rgb);
                }
                None
            }
            _ => None,
        };
        if let Some(encoding) = encoding {
            scan.value(range(&text, value), encoding);
        }
    }
}

fn texture(scan: &mut Scan<'_>, text: &str, section: &str) {
    let fields = || {
        section
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.trim(), value.trim()))
    };
    let kind = fields()
        .find(|(key, _)| *key == "type")
        .map(|(_, value)| value);
    let monochrome = fields()
        .find(|(key, _)| *key == "monochrome")
        .is_none_or(|(_, value)| !matches!(value, "no" | "false" | "0"));
    for (key, value) in fields() {
        if matches!(kind, Some("gradient_map" | "10")) && key == "stops" {
            let mut stops = value.split(';');
            if !matches!(stops.next().map(str::trim), Some("custom" | "0")) {
                continue;
            }
            for stop in stops {
                if let Some((_, color)) = stop.split_once(',') {
                    scan.value(range(text, color.trim()), Encoding::Rgb);
                }
            }
        } else if matches!(kind, Some("vector_graphics" | "29")) && key == "shapes" {
            for shape in value.split(';') {
                let fields: Vec<_> = shape.split(',').map(str::trim).collect();
                let (coordinates, colors) = match fields.first().copied() {
                    Some("line") => (4, 1),
                    Some("bezier") => (8, 1),
                    Some("rectangle" | "ellipse") => (4, 2),
                    _ => continue,
                };
                if fields.len() != coordinates + colors + 2 {
                    continue;
                }
                for color in &fields[coordinates + 1..=coordinates + colors] {
                    if super::number(color) == Some(u32::MAX) {
                        continue;
                    }
                    scan.value(
                        range(text, color),
                        if monochrome {
                            Encoding::Gray12
                        } else {
                            Encoding::Rgb
                        },
                    );
                }
            }
        }
    }
}

fn range(text: &str, value: &str) -> Range<usize> {
    let start = value.as_ptr() as usize - text.as_ptr() as usize;
    start..start + value.len()
}
