//! User-editable style. Omitted fields use the built-in palette and dimensions.

pub const TEMPLATE: &str = include_str!("../style.ini");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub background: u32,
    pub border: u32,
    pub hover: u32,
    pub text: u32,
    pub muted_text: u32,
    pub icon_size: i32,
    pub icon_padding: i32,
    pub padding: i32,
    pub corner_radius: i32,
    pub edge_margin: i32,
    pub trigger_size: i32,
    pub font_size: i32,
    pub opacity: i32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: 0x211a17,
            border: 0x463a33,
            hover: 0x40322b,
            text: 0xf3e6dc,
            muted_text: 0xc5b1a3,
            icon_size: 20,
            icon_padding: 7,
            padding: 3,
            corner_radius: 9,
            edge_margin: 8,
            trigger_size: 11,
            font_size: 13,
            opacity: 100,
        }
    }
}

impl Style {
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut style = Self::default();
        for (index, line) in source.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with([';', '#']) {
                continue;
            }
            let error = |message| format!("Line {}: {message}", index + 1);
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| error("expected name = value"))?;
            let (key, value) = (key.trim(), value.trim());
            let color = || parse_color(value).ok_or_else(|| error("expected a #RRGGBB color"));
            let number = |min, max| {
                value
                    .parse()
                    .ok()
                    .filter(|n| (min..=max).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "Line {}: {key} needs a whole number from {min} to {max}",
                            index + 1
                        )
                    })
            };
            match key {
                "background" => style.background = color()?,
                "border" => style.border = color()?,
                "hover" => style.hover = color()?,
                "text" => style.text = color()?,
                "muted_text" => style.muted_text = color()?,
                "icon_size" => style.icon_size = number(12, 64)?,
                "icon_padding" => style.icon_padding = number(2, 24)?,
                "padding" => style.padding = number(0, 24)?,
                "corner_radius" => style.corner_radius = number(0, 32)?,
                "edge_margin" => style.edge_margin = number(0, 80)?,
                "trigger_size" => style.trigger_size = number(6, 32)?,
                "font_size" => style.font_size = number(9, 28)?,
                "opacity" => style.opacity = number(30, 100)?,
                _ => return Err(format!("Line {}: unknown style setting '{key}'", index + 1)),
            }
        }
        Ok(style)
    }
}

fn parse_color(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    // Win32 COLORREF stores RGB in little-endian channel order.
    Some((rgb & 0xff) << 16 | (rgb & 0xff00) | rgb >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_matches_built_in_style_and_overrides_are_optional() {
        assert!(!TEMPLATE.is_empty());
        assert_eq!(Style::parse(TEMPLATE).unwrap(), Style::default());
        let style =
            Style::parse("\u{feff}; custom style\r\nbackground = #123456\r\nicon_size = 24")
                .unwrap();
        assert_eq!(style.background, 0x56_34_12);
        assert_eq!(style.icon_size, 24);
        assert_eq!(style.opacity, 100);
    }

    #[test]
    fn reports_invalid_theme_values() {
        assert!(
            Style::parse("background = #abczzz")
                .unwrap_err()
                .contains("Line 1")
        );
        assert!(
            Style::parse("\nopacity = 0")
                .unwrap_err()
                .contains("Line 2")
        );
        assert!(
            Style::parse("border_width = 3")
                .unwrap_err()
                .contains("unknown")
        );
    }
}
