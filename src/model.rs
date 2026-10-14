//! Persistent data: boxes ("zones"), their styles and the app settings.
//! Everything here is platform independent and serialized to config.json.

use crate::geom::Rect;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }

    pub fn from_hex(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// Relative luminance (0 = black, 1 = white).
    pub fn luminance(self) -> f32 {
        fn lin(c: u8) -> f32 {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::from_hex(&s).ok_or_else(|| serde::de::Error::custom("expected a #RRGGBB color"))
    }
}

pub const PASTEL_GREEN: Rgb = Rgb(0x77, 0xDD, 0x77);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleStyle {
    Plain,
    #[default]
    Outline,
    Chip,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BorderStyle {
    #[default]
    Solid,
    Dashed,
}

/// Visual style of a box. Sizes are in device-independent pixels (100% scaling).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoneStyle {
    pub color: Rgb,
    /// Fill opacity in percent.
    pub fill: u8,
    pub border_width: f32,
    pub border_style: BorderStyle,
    pub radius: f32,
    pub title_v: VAlign,
    pub title_h: HAlign,
    pub title_size: f32,
    pub title_style: TitleStyle,
}

impl Default for ZoneStyle {
    fn default() -> Self {
        ZoneStyle {
            color: PASTEL_GREEN,
            fill: 10,
            border_width: 2.0,
            border_style: BorderStyle::Solid,
            radius: 12.0,
            title_v: VAlign::Top,
            title_h: HAlign::Left,
            title_size: 16.0,
            title_style: TitleStyle::Outline,
        }
    }
}

/// A labeled box drawn on one monitor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub id: u64,
    /// Stable identifier of the monitor this box belongs to.
    pub monitor: String,
    /// Hardware id of that monitor (model code), used to find it again if its port changes.
    #[serde(default)]
    pub monitor_hw: String,
    /// Position relative to the monitor's top-left corner, in physical pixels at `ref_size`.
    pub rect: Rect,
    /// Monitor resolution when `rect` was saved; used to rescale after resolution changes.
    pub ref_size: (u32, u32),
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub style: ZoneStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DotColor {
    #[default]
    White,
    Black,
}

impl DotColor {
    pub fn rgb(self) -> Rgb {
        match self {
            DotColor::White => Rgb(0xFF, 0xFF, 0xFF),
            DotColor::Black => Rgb(0x10, 0x10, 0x10),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridSettings {
    /// Show the dot grid on the desktop all the time (it always shows while editing).
    pub show_on_desktop: bool,
    pub color: DotColor,
    /// Dot radius in device-independent pixels.
    pub size: f32,
    /// Opacity in percent.
    pub opacity: u8,
}

impl Default for GridSettings {
    fn default() -> Self {
        GridSettings {
            show_on_desktop: false,
            color: DotColor::White,
            size: 1.5,
            opacity: 75,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub zones: Vec<Zone>,
    pub show_zones: bool,
    /// Snap boxes to the desktop icon grid (hold Alt while dragging to invert).
    pub snap: bool,
    pub grid: GridSettings,
    /// Style used for new boxes.
    pub defaults: ZoneStyle,
    /// Key of the title font (see the platform font list).
    pub font: String,
    pub hotkey: bool,
    pub minimize_windows: bool,
    pub custom_colors: Vec<Rgb>,
    pub welcomed: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: 1,
            zones: Vec::new(),
            show_zones: true,
            snap: true,
            grid: GridSettings::default(),
            defaults: ZoneStyle::default(),
            font: "segoe-semibold".into(),
            hotkey: true,
            minimize_windows: true,
            custom_colors: Vec::new(),
            welcomed: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Option lists shown in menus. Shared so every platform offers the same choices.

pub const PALETTE: &[(&str, Rgb)] = &[
    ("Pastel green", PASTEL_GREEN),
    ("Mint", Rgb(0x8F, 0xE3, 0xCF)),
    ("Sky", Rgb(0x8E, 0xC5, 0xFF)),
    ("Lavender", Rgb(0xC3, 0xB1, 0xE1)),
    ("Pink", Rgb(0xFF, 0xB3, 0xC7)),
    ("Peach", Rgb(0xFF, 0xC8, 0x9E)),
    ("Lemon", Rgb(0xFD, 0xF2, 0x8C)),
    ("Coral", Rgb(0xFF, 0x7F, 0x6E)),
    ("Chalk white", Rgb(0xF4, 0xF4, 0xF0)),
    ("Slate", Rgb(0x70, 0x80, 0x90)),
    ("Charcoal", Rgb(0x2B, 0x2B, 0x2B)),
];
pub const TITLE_SIZES: &[(&str, f32)] = &[
    ("Small", 13.0),
    ("Medium", 16.0),
    ("Large", 21.0),
    ("Huge", 28.0),
];
pub const TITLE_STYLES: &[(&str, TitleStyle)] = &[
    ("Plain text", TitleStyle::Plain),
    ("Outlined text", TitleStyle::Outline),
    ("Label chip", TitleStyle::Chip),
];
pub const RADII: &[(&str, f32)] = &[
    ("Square", 0.0),
    ("4 px", 4.0),
    ("8 px", 8.0),
    ("12 px", 12.0),
    ("16 px", 16.0),
    ("24 px", 24.0),
    ("32 px", 32.0),
];
pub const BORDER_WIDTHS: &[(&str, f32)] = &[
    ("No border", 0.0),
    ("1 px", 1.0),
    ("2 px", 2.0),
    ("3 px", 3.0),
    ("4 px", 4.0),
    ("6 px", 6.0),
];
pub const BORDER_STYLES: &[(&str, BorderStyle)] =
    &[("Solid", BorderStyle::Solid), ("Dashed", BorderStyle::Dashed)];
pub const FILLS: &[(&str, u8)] = &[
    ("No fill", 0),
    ("5%", 5),
    ("10%", 10),
    ("15%", 15),
    ("25%", 25),
    ("40%", 40),
    ("60%", 60),
];
pub const VALIGNS: &[(&str, VAlign)] = &[
    ("Top", VAlign::Top),
    ("Middle", VAlign::Middle),
    ("Bottom", VAlign::Bottom),
];
pub const HALIGNS: &[(&str, HAlign)] = &[
    ("left", HAlign::Left),
    ("center", HAlign::Center),
    ("right", HAlign::Right),
];
pub const DOT_SIZES: &[(&str, f32)] = &[
    ("Tiny dots", 1.0),
    ("Small dots", 1.5),
    ("Medium dots", 2.0),
    ("Large dots", 3.0),
];
pub const DOT_OPACITY: &[(&str, u8)] = &[
    ("25% opacity", 25),
    ("50% opacity", 50),
    ("75% opacity", 75),
    ("100% opacity", 100),
];
pub const ICON_SIZES: &[(&str, i32)] = &[
    ("Small (32 px)", 32),
    ("Medium (48 px)", 48),
    ("Medium-large (64 px)", 64),
    ("Large (96 px)", 96),
    ("Extra large (128 px)", 128),
];

// Style commands are encoded as small offsets so a menu can address any style field.
const OFF_COLOR: u32 = 0;
pub const OFF_CUSTOM_COLOR: u32 = 20;
const OFF_POS: u32 = 30;
const OFF_TSIZE: u32 = 40;
const OFF_TSTYLE: u32 = 50;
const OFF_RADIUS: u32 = 60;
const OFF_BORDER: u32 = 70;
const OFF_BSTYLE: u32 = 80;
const OFF_FILL: u32 = 90;
/// Style offsets are always below this value.
pub const STYLE_SPAN: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StyleCmd {
    Color(Rgb),
    CustomColor,
    TitlePos(VAlign, HAlign),
    TitleSize(f32),
    TitleStyle(TitleStyle),
    Radius(f32),
    BorderWidth(f32),
    BorderStyle(BorderStyle),
    Fill(u8),
}

/// Lists every menu entry of a style editor as (submenu, offset, label, checked).
pub struct StyleMenu {
    pub sections: Vec<(&'static str, Vec<StyleEntry>)>,
}

pub struct StyleEntry {
    pub offset: u32,
    pub label: String,
    pub checked: bool,
    pub color: Option<Rgb>,
    /// Draw a separator before this entry.
    pub separator: bool,
}

impl StyleCmd {
    pub fn decode(off: u32) -> Option<StyleCmd> {
        let idx = |base: u32| (off - base) as usize;
        Some(match off {
            o if o < OFF_COLOR + 16 => StyleCmd::Color(PALETTE.get(idx(OFF_COLOR))?.1),
            OFF_CUSTOM_COLOR => StyleCmd::CustomColor,
            o if (OFF_POS..OFF_POS + 9).contains(&o) => {
                let i = idx(OFF_POS);
                StyleCmd::TitlePos(VALIGNS[i / 3].1, HALIGNS[i % 3].1)
            }
            o if (OFF_TSIZE..OFF_TSIZE + 10).contains(&o) => {
                StyleCmd::TitleSize(TITLE_SIZES.get(idx(OFF_TSIZE))?.1)
            }
            o if (OFF_TSTYLE..OFF_TSTYLE + 10).contains(&o) => {
                StyleCmd::TitleStyle(TITLE_STYLES.get(idx(OFF_TSTYLE))?.1)
            }
            o if (OFF_RADIUS..OFF_RADIUS + 10).contains(&o) => {
                StyleCmd::Radius(RADII.get(idx(OFF_RADIUS))?.1)
            }
            o if (OFF_BORDER..OFF_BORDER + 10).contains(&o) => {
                StyleCmd::BorderWidth(BORDER_WIDTHS.get(idx(OFF_BORDER))?.1)
            }
            o if (OFF_BSTYLE..OFF_BSTYLE + 10).contains(&o) => {
                StyleCmd::BorderStyle(BORDER_STYLES.get(idx(OFF_BSTYLE))?.1)
            }
            o if (OFF_FILL..OFF_FILL + 10).contains(&o) => StyleCmd::Fill(FILLS.get(idx(OFF_FILL))?.1),
            _ => return None,
        })
    }
}

impl ZoneStyle {
    pub fn apply(&mut self, cmd: StyleCmd) {
        match cmd {
            StyleCmd::Color(c) => self.color = c,
            StyleCmd::CustomColor => {}
            StyleCmd::TitlePos(v, h) => {
                self.title_v = v;
                self.title_h = h;
            }
            StyleCmd::TitleSize(s) => self.title_size = s,
            StyleCmd::TitleStyle(s) => self.title_style = s,
            StyleCmd::Radius(r) => self.radius = r,
            StyleCmd::BorderWidth(w) => self.border_width = w,
            StyleCmd::BorderStyle(s) => self.border_style = s,
            StyleCmd::Fill(f) => self.fill = f,
        }
    }

    /// Describes the style submenus with the current values checked.
    pub fn menu(&self) -> StyleMenu {
        let entry = |offset: u32, label: String, checked: bool| StyleEntry {
            offset,
            label,
            checked,
            color: None,
            separator: false,
        };
        let mut colors: Vec<StyleEntry> = PALETTE
            .iter()
            .enumerate()
            .map(|(i, (name, c))| StyleEntry {
                color: Some(*c),
                ..entry(OFF_COLOR + i as u32, (*name).into(), *c == self.color)
            })
            .collect();
        let custom = !PALETTE.iter().any(|(_, c)| *c == self.color);
        colors.push(StyleEntry {
            separator: true,
            color: custom.then_some(self.color),
            ..entry(OFF_CUSTOM_COLOR, "Custom color…".into(), custom)
        });

        let mut pos = Vec::new();
        for (vi, (vn, v)) in VALIGNS.iter().enumerate() {
            for (hi, (hn, h)) in HALIGNS.iter().enumerate() {
                let label = if *v == VAlign::Middle && *h == HAlign::Center {
                    "Center".to_string()
                } else {
                    format!("{vn} {hn}")
                };
                pos.push(StyleEntry {
                    separator: hi == 0 && vi > 0,
                    ..entry(
                        OFF_POS + (vi * 3 + hi) as u32,
                        label,
                        self.title_v == *v && self.title_h == *h,
                    )
                });
            }
        }

        let mut title: Vec<StyleEntry> = TITLE_SIZES
            .iter()
            .enumerate()
            .map(|(i, (n, s))| entry(OFF_TSIZE + i as u32, (*n).into(), (*s - self.title_size).abs() < 0.1))
            .collect();
        for (i, (n, s)) in TITLE_STYLES.iter().enumerate() {
            title.push(StyleEntry {
                separator: i == 0,
                ..entry(OFF_TSTYLE + i as u32, (*n).into(), *s == self.title_style)
            });
        }

        let radius = RADII
            .iter()
            .enumerate()
            .map(|(i, (n, r))| entry(OFF_RADIUS + i as u32, (*n).into(), (*r - self.radius).abs() < 0.1))
            .collect();

        let mut border: Vec<StyleEntry> = BORDER_WIDTHS
            .iter()
            .enumerate()
            .map(|(i, (n, w))| {
                entry(OFF_BORDER + i as u32, (*n).into(), (*w - self.border_width).abs() < 0.1)
            })
            .collect();
        for (i, (n, s)) in BORDER_STYLES.iter().enumerate() {
            border.push(StyleEntry {
                separator: i == 0,
                ..entry(OFF_BSTYLE + i as u32, (*n).into(), *s == self.border_style)
            });
        }

        let fill = FILLS
            .iter()
            .enumerate()
            .map(|(i, (n, f))| entry(OFF_FILL + i as u32, (*n).into(), *f == self.fill))
            .collect();

        StyleMenu {
            sections: vec![
                ("Color", colors),
                ("Title position", pos),
                ("Title size and style", title),
                ("Corner roundness", radius),
                ("Border", border),
                ("Fill", fill),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_offsets_round_trip() {
        let menu = ZoneStyle::default().menu();
        for (_, entries) in &menu.sections {
            for e in entries {
                assert!(e.offset < STYLE_SPAN);
                assert!(StyleCmd::decode(e.offset).is_some(), "offset {}", e.offset);
            }
        }
        assert_eq!(
            StyleCmd::decode(OFF_POS + 8),
            Some(StyleCmd::TitlePos(VAlign::Bottom, HAlign::Right))
        );
    }

    #[test]
    fn config_round_trip() {
        let mut cfg = Config::default();
        cfg.zones.push(Zone {
            id: 7,
            monitor: "m".into(),
            monitor_hw: "hw".into(),
            rect: Rect::new(1.0, 2.0, 3.0, 4.0),
            ref_size: (1920, 1080),
            title: "Inbox".into(),
            style: ZoneStyle::default(),
        });
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.zones, cfg.zones);
        // Unknown and missing fields are tolerated.
        let partial: Config = serde_json::from_str(r#"{"snap": false, "future": 1}"#).unwrap();
        assert!(!partial.snap);
        assert!(partial.show_zones);
    }
}
