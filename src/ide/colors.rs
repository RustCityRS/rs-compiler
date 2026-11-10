use super::fields::Value;
use super::{
    Color, DocumentColor, Index, Location, Symbol, SymbolKind, TextEdit, TokenKind, config, index,
};
use crate::project::{Document, Project, SourceId};
use crate::symbol::SymbolKind as RegistryKind;
use crate::token::Kind;
use crate::types::Type;
use std::ops::Range;

mod extra;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Encoding {
    Rgb,
    Argb,
    Rgb15,
    Hsl,
    Hsl32,
    Gray12,
}

struct Swatch {
    range: Range<usize>,
    color: Color,
    encoding: Encoding,
    value: u32,
    editable: bool,
    tag: bool,
}

impl Project {
    pub fn document_colors(&self, source: SourceId) -> Vec<DocumentColor> {
        self.color_swatches(source)
            .into_iter()
            .map(|swatch| DocumentColor {
                range: swatch.range,
                color: swatch.color,
            })
            .collect()
    }

    pub fn color_presentation(
        &self,
        source: SourceId,
        range: Range<usize>,
        color: Color,
    ) -> Option<TextEdit> {
        if [color.red, color.green, color.blue, color.alpha]
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return None;
        }
        let swatch = self
            .color_swatches(source)
            .into_iter()
            .find(|swatch| swatch.range == range && swatch.editable)?;
        let document = self.document(source)?;
        let original = &document.text[range.clone()];
        let key = document.text[..range.start]
            .rsplit('\n')
            .next()
            .and_then(|line| line.split_once('='))
            .map(|(key, _)| key.trim());
        let signed = matches!(
            (document.config.as_deref(), key),
            (Some("particleemitter"), Some("rgba" | "rgbatarget"))
                | (Some("npc"), Some("spotshadowcolour"))
        );
        let value = if color == swatch.color {
            swatch.value
        } else {
            swatch.encoding.encode(color)
        };
        let value = if swatch.encoding == Encoding::Rgb {
            value | swatch.value & 0xff000000
        } else {
            value
        };
        let replacement = if color == swatch.color {
            original.to_string()
        } else if swatch.tag || original.starts_with("0x") || original.starts_with("0X") {
            let (prefix, digits) = if swatch.tag {
                ("", original)
            } else {
                original.split_at(2)
            };
            let width = digits.bytes().filter(|byte| *byte != b'_').count();
            if digits.bytes().any(|byte| byte.is_ascii_uppercase()) {
                format!("{prefix}{value:0width$X}")
            } else {
                format!("{prefix}{value:0width$x}")
            }
        } else if original.starts_with('-') || signed {
            if swatch.encoding == Encoding::Hsl {
                (value as i16).to_string()
            } else {
                (value as i32).to_string()
            }
        } else {
            value.to_string()
        };
        Some(TextEdit {
            location: Location { source, range },
            replacement,
        })
    }

    fn color_swatches(&self, source: SourceId) -> Vec<Swatch> {
        let Some(document) = self.document(source) else {
            return Vec::new();
        };
        let mut scan = Scan {
            project: self,
            document,
            index: self.symbol_index(),
            out: Vec::new(),
        };
        if document.config.is_some() {
            scan.config();
        } else {
            scan.script();
        }
        scan.out
            .sort_by_key(|swatch| (swatch.range.start, swatch.range.end));
        scan.out.dedup_by(|a, b| a.range == b.range);
        scan.out
    }
}

struct Scan<'a> {
    project: &'a Project,
    document: &'a Document,
    index: &'a Index,
    out: Vec<Swatch>,
}

impl Scan<'_> {
    fn hex(&mut self, range: Range<usize>, encoding: Encoding) {
        let Ok(value) = u32::from_str_radix(&self.document.text[range.clone()], 16) else {
            return;
        };
        self.out.push(Swatch {
            range,
            color: encoding.decode(value),
            encoding,
            value,
            editable: true,
            tag: true,
        });
    }

    fn value(&mut self, range: Range<usize>, encoding: Encoding) {
        let text = &self.document.text[range.clone()];
        let start = range.start + text.len() - text.trim_start().len();
        let range = start..start + text.trim().len();
        let text = &self.document.text[range.clone()];
        let editable = !text.starts_with('^');
        let Some(value) = self.resolve(text, 0) else {
            return;
        };
        if matches!(encoding, Encoding::Rgb15) && value > 0x7fff
            || encoding == Encoding::Hsl
                && value > 0xffff
                && !(i16::MIN as i32..=-1).contains(&(value as i32))
        {
            return;
        }
        self.out.push(Swatch {
            range,
            color: encoding.decode(value),
            encoding,
            value,
            editable,
            tag: false,
        });
    }

    fn resolve(&self, text: &str, depth: usize) -> Option<u32> {
        if depth > 16 {
            return None;
        }
        if let Some(name) = text.strip_prefix('^') {
            if let Some(id) = self.index.configs.keys.get(&config::key("constant", name)) {
                let symbol = &self.index.configs.symbols[*id];
                if let Some(definition) = &symbol.definition {
                    let document = self.project.document(definition.source)?;
                    let line = document.text[definition.range.end..].lines().next()?;
                    let (_, value) = line.split_once('=')?;
                    return self.resolve(value.split("//").next()?.trim(), depth + 1);
                }
            }
            return match &self.project.registry.lookup_constant(name)?.kind {
                RegistryKind::Constant { int_value, .. } => int_value.map(|value| value as u32),
                _ => None,
            };
        }
        number(text)
    }

    fn config(&mut self) {
        let kind = self.document.config.as_deref().unwrap();
        let tokens = config::typed_tokens(self.document, self.index.config_types());
        let color_file = self
            .document
            .label
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches!(name, "colour" | "color" | "colours" | "colors"));
        let mut constant = None;
        for typed in tokens {
            let token = typed.token;
            let text = &self.document.text[token.range.clone()];
            if kind == "constant" && token.declaration {
                constant = named_encoding(text).or(color_file.then_some(Encoding::Rgb));
            }
            if token.kind == TokenKind::String {
                self.tags(token.range.clone());
            }
            if matches!(
                token.kind,
                TokenKind::Number | TokenKind::Variable | TokenKind::EnumMember
            ) && !token.declaration
            {
                if let Some(Value::Color(encoding)) = typed.value {
                    if kind == "wma" && self.resolve(text, 0) == Some(u32::MAX) {
                        continue;
                    }
                    if matches!(kind, "if" | "flo" | "flu" | "texture")
                        && encoding == Encoding::Rgb
                        && !text.starts_with(['^', '%'])
                        && !text.starts_with("0x")
                        && !text.starts_with("0X")
                    {
                        self.hex(token.range, encoding);
                    } else {
                        self.value(token.range, encoding);
                    }
                } else if kind == "constant"
                    && let Some(encoding) = constant
                {
                    self.value(token.range, encoding);
                }
            }
        }
        extra::scan(self);
    }

    fn script(&mut self) {
        for (token, range) in self.document.tokens.iter().zip(&self.document.spans) {
            if matches!(token.kind, Kind::StringLiteral | Kind::RawStringLiteral) {
                self.tags(range.clone());
            }
        }
        let hooks = self
            .index
            .hooks
            .get(&self.document.id)
            .map_or(&[][..], Vec::as_slice);
        let tokens = index::tokens(self.document, &|name| self.index.is_compound(name), hooks);
        let tokens = tokens
            .iter()
            .filter(|(token, _)| {
                !matches!(token.kind, Kind::SingleLineComment | Kind::MultiLineComment)
            })
            .collect::<Vec<_>>();
        let scripts = self
            .index
            .references_in(self.document.id)
            .filter(|reference| reference.declaration)
            .map(|reference| self.index.symbol(reference.symbol))
            .filter(|symbol| {
                matches!(
                    symbol.kind,
                    SymbolKind::Procedure | SymbolKind::ClientScript | SymbolKind::Trigger
                )
            })
            .collect::<Vec<_>>();
        let mut stack: Vec<(Vec<Option<Encoding>>, usize, usize)> = Vec::new();
        for (i, (token, range)) in tokens.iter().enumerate() {
            if token.kind == Kind::LParen {
                let previous = i.checked_sub(1).map(|i| tokens[i]);
                let symbol = previous.and_then(|(_, range)| self.symbol(range.start));
                let encodings = if previous.is_some_and(|(token, _)| token.kind == Kind::Return) {
                    scripts
                        .iter()
                        .find(|symbol| {
                            symbol.container.is_none()
                                && symbol.extent.as_ref().is_some_and(|extent| {
                                    extent.source == self.document.id
                                        && extent.range.contains(&range.start)
                                })
                        })
                        .map(|symbol| {
                            symbol
                                .returns
                                .iter()
                                .map(|ty| (*ty == Type::Colour).then_some(Encoding::Rgb))
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    symbol
                        .filter(|symbol| {
                            matches!(
                                symbol.kind,
                                SymbolKind::Command
                                    | SymbolKind::Procedure
                                    | SymbolKind::ClientScript
                                    | SymbolKind::Trigger
                            )
                        })
                        .map(|symbol| {
                            symbol
                                .parameters
                                .iter()
                                .enumerate()
                                .map(|(argument, (_, ty))| {
                                    if *ty == Type::Colour {
                                        Some(Encoding::Rgb)
                                    } else {
                                        command_encoding(symbol, argument)
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                };
                stack.push((encodings, 0, range.end));
            } else if matches!(token.kind, Kind::Comma | Kind::RParen) {
                if let Some((encodings, argument, start)) = stack.last_mut() {
                    if let Some(Some(encoding)) = encodings.get(*argument) {
                        let first = tokens.partition_point(|(_, range)| range.start < *start);
                        if first < i {
                            self.value(tokens[first].1.start..tokens[i - 1].1.end, *encoding);
                        }
                    }
                    *argument += 1;
                    *start = range.end;
                }
                if token.kind == Kind::RParen {
                    stack.pop();
                }
            } else if token.kind == Kind::Equals || token.kind == Kind::ComparisonOperator {
                let previous = i.checked_sub(1).map(|i| tokens[i]);
                if previous
                    .and_then(|(_, range)| self.symbol(range.start))
                    .is_some_and(|symbol| symbol.value_type == Some(Type::Colour))
                    && let Some((next, next_range)) = tokens.get(i + 1)
                {
                    let end = if next.kind == Kind::ConstantVar
                        || next.kind == Kind::BinaryOperator && next.value == "-"
                    {
                        tokens.get(i + 2).map(|(_, range)| range.end)
                    } else if next.kind == Kind::Number {
                        Some(next_range.end)
                    } else {
                        None
                    };
                    if let Some(end) = end {
                        self.value(next_range.start..end, Encoding::Rgb);
                    }
                }
            } else if token.kind == Kind::ConstantVar {
                let Some((_, name_range)) = tokens.get(i + 1) else {
                    continue;
                };
                let symbol = self.symbol(name_range.start);
                let encoding = symbol.and_then(|symbol| {
                    if symbol.value_type == Some(Type::Colour) {
                        Some(Encoding::Rgb)
                    } else {
                        named_encoding(&symbol.name).or_else(|| {
                            let source =
                                self.project.document(symbol.definition.as_ref()?.source)?;
                            matches!(
                                source.label.file_stem()?.to_str()?,
                                "colour" | "color" | "colours" | "colors"
                            )
                            .then_some(Encoding::Rgb)
                        })
                    }
                });
                if let Some(encoding) = encoding {
                    self.value(range.start..name_range.end, encoding);
                }
            }
        }
    }

    fn symbol(&self, offset: usize) -> Option<&Symbol> {
        self.index
            .at(self.document.id, offset)
            .map(|reference| self.index.symbol(reference.symbol))
    }

    fn tags(&mut self, range: Range<usize>) {
        let text = &self.document.text[range.clone()];
        for prefix in ["<col=", "<shad=", "<str=", "<u="] {
            for (offset, _) in text.match_indices(prefix) {
                let start = offset + prefix.len();
                let Some(end) = text[start..].find('>') else {
                    continue;
                };
                let value = &text[start..start + end];
                if value.is_empty()
                    || value.len() > 6
                    || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    continue;
                }
                let value = u32::from_str_radix(value, 16).unwrap();
                self.out.push(Swatch {
                    range: range.start + start..range.start + start + end,
                    color: Encoding::Rgb.decode(value),
                    encoding: Encoding::Rgb,
                    value,
                    editable: true,
                    tag: true,
                });
            }
        }
        let mut tag_end = 0;
        for (offset, _) in text.match_indices('@') {
            if offset < tag_end {
                continue;
            }
            let Some(tag) = text
                .get(offset..offset + 5)
                .filter(|tag| tag.ends_with('@'))
            else {
                continue;
            };
            let value = match &tag[1..4] {
                "red" => 0xff0000,
                "gre" => 0x00ff00,
                "blu" => 0x0000ff,
                "yel" => 0xffff00,
                "cya" => 0x00ffff,
                "mag" => 0xff00ff,
                "whi" => 0xffffff,
                "bla" => 0,
                "lre" => 0xff9040,
                "dre" => 0x800000,
                "dbl" => 0x000080,
                "or1" => 0xffb000,
                "or2" => 0xff7000,
                "or3" => 0xff3000,
                "gr1" => 0xc0ff00,
                "gr2" => 0x80ff00,
                "gr3" => 0x40ff00,
                _ => continue,
            };
            tag_end = offset + 5;
            self.out.push(Swatch {
                range: range.start + offset..range.start + offset + 5,
                color: Encoding::Rgb.decode(value),
                encoding: Encoding::Rgb,
                value,
                editable: false,
                tag: false,
            });
        }
    }
}

fn named_encoding(name: &str) -> Option<Encoding> {
    match name.rsplit('_').next()? {
        "rgb" => Some(Encoding::Rgb),
        "argb" => Some(Encoding::Argb),
        "rgb15" => Some(Encoding::Rgb15),
        "hsl" => Some(Encoding::Hsl),
        _ => None,
    }
}

fn command_encoding(symbol: &Symbol, argument: usize) -> Option<Encoding> {
    if symbol.kind != SymbolKind::Command {
        return None;
    }
    match (symbol.name.as_str(), argument) {
        ("if_setcolour", 1)
            if symbol
                .parameters
                .first()
                .is_some_and(|(_, ty)| *ty == Type::Component) =>
        {
            Some(Encoding::Rgb)
        }
        ("text_coord", 2) => Some(Encoding::Rgb),
        ("if_setrecol" | "cc_setrecol" | "op1131", 1 | 2) => Some(Encoding::Hsl),
        ("op5401", 1) => Some(Encoding::Rgb),
        ("hsvtorgb", 0) => Some(Encoding::Hsl),
        _ => None,
    }
}

fn number(text: &str) -> Option<u32> {
    let value = text.replace('_', "");
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        value
            .parse::<i64>()
            .ok()
            .filter(|value| (i32::MIN as i64..=u32::MAX as i64).contains(value))
            .map(|value| value as u32)
    }
}

impl Encoding {
    fn decode(self, value: u32) -> Color {
        match self {
            Self::Rgb | Self::Argb => Color {
                red: ((value >> 16) & 255) as f64 / 255.0,
                green: ((value >> 8) & 255) as f64 / 255.0,
                blue: (value & 255) as f64 / 255.0,
                alpha: if self == Self::Argb {
                    (value >> 24) as f64 / 255.0
                } else {
                    1.0
                },
            },
            Self::Rgb15 => Color {
                red: ((value >> 10) & 31) as f64 / 31.0,
                green: ((value >> 5) & 31) as f64 / 31.0,
                blue: (value & 31) as f64 / 31.0,
                alpha: 1.0,
            },
            Self::Gray12 => {
                let gray = (value as f64 / 4096.0).min(1.0);
                Color {
                    red: gray,
                    green: gray,
                    blue: gray,
                    alpha: 1.0,
                }
            }
            Self::Hsl | Self::Hsl32 => {
                let (hue, saturation, lightness, gamma) = if self == Self::Hsl {
                    (
                        ((value >> 10) & 63) as f64 / 64.0 + 1.0 / 128.0,
                        ((value >> 7) & 7) as f64 / 8.0 + 1.0 / 16.0,
                        (value & 127) as f64 / 128.0,
                        0.7,
                    )
                } else {
                    (
                        ((value as i32 >> 20) as f64 * 3.0 / 256.0).rem_euclid(1.0),
                        (((value >> 10) & 1023) as f64 * 3.0 / 256.0).min(1.0),
                        ((value & 1023) as f64 * 3.0 / 256.0).min(1.0),
                        1.0,
                    )
                };
                let q = if lightness < 0.5 {
                    lightness * (1.0 + saturation)
                } else {
                    lightness + saturation - lightness * saturation
                };
                let p = 2.0 * lightness - q;
                let channel = |offset: f64| {
                    let h = (hue + offset).rem_euclid(1.0);
                    let value = if h < 1.0 / 6.0 {
                        p + (q - p) * 6.0 * h
                    } else if h < 0.5 {
                        q
                    } else if h < 2.0 / 3.0 {
                        p + (q - p) * (2.0 / 3.0 - h) * 6.0
                    } else {
                        p
                    };
                    value.powf(gamma)
                };
                Color {
                    red: channel(1.0 / 3.0),
                    green: channel(0.0),
                    blue: channel(-1.0 / 3.0),
                    alpha: 1.0,
                }
            }
        }
    }

    fn encode(self, color: Color) -> u32 {
        let channel = |value: f64, max: f64| (value * max).round() as u32;
        match self {
            Self::Rgb | Self::Argb => {
                channel(color.red, 255.0) << 16
                    | channel(color.green, 255.0) << 8
                    | channel(color.blue, 255.0)
                    | if self == Self::Argb {
                        channel(color.alpha, 255.0) << 24
                    } else {
                        0
                    }
            }
            Self::Rgb15 => {
                channel(color.red, 31.0) << 10
                    | channel(color.green, 31.0) << 5
                    | channel(color.blue, 31.0)
            }
            Self::Gray12 => channel(
                color.red * 0.2126 + color.green * 0.7152 + color.blue * 0.0722,
                4096.0,
            ),
            Self::Hsl | Self::Hsl32 => {
                let [r, g, b] = [color.red, color.green, color.blue].map(|value| {
                    if self == Self::Hsl {
                        value.powf(1.0 / 0.7)
                    } else {
                        value * 255.0 / 256.0
                    }
                });
                let min = r.min(g).min(b);
                let max = r.max(g).max(b);
                let lightness = (min + max) / 2.0;
                let delta = max - min;
                let saturation = if delta == 0.0 {
                    0.0
                } else {
                    delta / (1.0 - (2.0 * lightness - 1.0).abs())
                };
                let hue = if delta == 0.0 {
                    0.0
                } else if max == r {
                    (g - b) / delta / 6.0
                } else if max == g {
                    ((b - r) / delta + 2.0) / 6.0
                } else {
                    ((r - g) / delta + 4.0) / 6.0
                };
                if self == Self::Hsl {
                    ((hue.rem_euclid(1.0) * 64.0) as u32).min(63) << 10
                        | ((saturation * 8.0) as u32).min(7) << 7
                        | channel(lightness, 128.0).min(127)
                } else {
                    let scale = |value: f64| (value * 256.0 / 3.0) as i32;
                    (scale(hue) << 20)
                        .wrapping_add(scale(saturation) << 10)
                        .wrapping_add(scale(lightness)) as u32
                }
            }
        }
    }
}
