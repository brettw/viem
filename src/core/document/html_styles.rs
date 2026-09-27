//! CSS serialization shared by styled HTML export and clipboard presentation.
use super::{
    html, BlockProperties, CharacterProperties, LineSpacing, ParagraphAlignment, WritingDirection,
};
fn direction(value: WritingDirection) -> &'static str {
    match value {
        WritingDirection::RightToLeft => "rtl",
        _ => "ltr",
    }
}

pub(super) fn block_css(properties: &BlockProperties) -> String {
    let mut out = String::new();
    if let Some(background) = properties.background {
        let declarations = html::character_css(&CharacterProperties {
            background: Some(background),
            ..Default::default()
        });
        out.push_str(&declarations);
        out.push_str("; ");
    }
    macro_rules! length {
        ($key:literal,$field:ident) => {
            if let Some(v) = properties.$field {
                out.push_str(&format!("{}: {v}pt; ", $key));
            }
        };
    }
    length!("margin-left", margin_left);
    length!("margin-right", margin_right);
    length!("margin-block-start", margin_top);
    length!("margin-block-end", margin_bottom);
    length!("margin-inline-start", leading_indent);
    length!("margin-inline-end", trailing_indent);
    length!("text-indent", first_line_indent);
    length!("padding-top", padding_top);
    length!("padding-right", padding_right);
    length!("padding-bottom", padding_bottom);
    length!("padding-left", padding_left);
    if let Some(v) = properties.border_top_width {
        out.push_str(&format!(
            "border-top-width: {v}pt; border-top-style: solid; "
        ));
    }
    if let Some(color) = properties.border_top_color {
        let css = html::character_css(&CharacterProperties {
            foreground: Some(color),
            ..Default::default()
        });
        out.push_str(&css.replacen("color:", "border-top-color:", 1));
        out.push_str("; ");
    }
    if let Some(v) = properties.border_right_width {
        out.push_str(&format!(
            "border-right-width: {v}pt; border-right-style: solid; "
        ));
    }
    if let Some(color) = properties.border_right_color {
        let css = html::character_css(&CharacterProperties {
            foreground: Some(color),
            ..Default::default()
        });
        out.push_str(&css.replacen("color:", "border-right-color:", 1));
        out.push_str("; ");
    }
    if let Some(v) = properties.border_bottom_width {
        out.push_str(&format!(
            "border-bottom-width: {v}pt; border-bottom-style: solid; "
        ));
    }
    if let Some(color) = properties.border_bottom_color {
        let css = html::character_css(&CharacterProperties {
            foreground: Some(color),
            ..Default::default()
        });
        out.push_str(&css.replacen("color:", "border-bottom-color:", 1));
        out.push_str("; ");
    }
    if let Some(v) = properties.border_left_width {
        out.push_str(&format!(
            "border-left-width: {v}pt; border-left-style: solid; "
        ));
    }
    if let Some(color) = properties.border_left_color {
        let css = html::character_css(&CharacterProperties {
            foreground: Some(color),
            ..Default::default()
        });
        out.push_str(&css.replacen("color:", "border-left-color:", 1));
        out.push_str("; ");
    }
    if let Some(v) = properties.alignment {
        out.push_str(&format!(
            "text-align: {}; ",
            match v {
                ParagraphAlignment::Start => "start",
                ParagraphAlignment::Center => "center",
                ParagraphAlignment::End => "end",
            }
        ));
    }
    if let Some(v) = properties.base_direction {
        if v != WritingDirection::Natural {
            out.push_str(&format!("direction: {}; ", direction(v)));
        }
    }
    if let Some(v) = properties.line_spacing {
        out.push_str(&format!(
            "line-height: {}; ",
            match v {
                LineSpacing::Normal => "normal".into(),
                LineSpacing::Multiplier(n) => n.to_string(),
                LineSpacing::AtLeast(n) | LineSpacing::Exact(n) => format!("{n}pt"),
            }
        ));
    }
    out
}
