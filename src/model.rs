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
pub const WHITE: Rgb = Rgb(0xFF, 0xFF, 0xFF);

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
    Left,
    #[default]
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
    /// Box color: the background, and the border and title unless they have their own.
    pub color: Rgb,
    /// Background opacity in percent.
    pub fill: u8,
    /// None: same as `color`.
    pub border_color: Option<Rgb>,
    /// Border opacity in percent.
    pub border_opacity: u8,
    pub border_width: f32,
    pub border_style: BorderStyle,
    pub radius: f32,
    /// Title sits on the border line (top or bottom) instead of inside the box.
    pub title_on_border: bool,
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
            border_color: None,
            border_opacity: 95,
            border_width: 2.0,
            border_style: BorderStyle::Solid,
            radius: 12.0,
            title_on_border: true,
            title_v: VAlign::Top,
            title_h: HAlign::Center,
            title_size: 16.0,
            title_style: TitleStyle::Outline,
        }
    }
}

impl ZoneStyle {
    /// Color of the border and the title.
    pub fn line_color(&self) -> Rgb {
        self.border_color.unwrap_or(self.color)
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

/// Accepts "#RRGGBB" plus the "white"/"black" names written by version 0.1.
fn dot_color<'de, D: Deserializer<'de>>(d: D) -> Result<Rgb, D::Error> {
    let s = String::deserialize(d)?;
    match s.as_str() {
        "white" => Ok(WHITE),
        "black" => Ok(Rgb(0x10, 0x10, 0x10)),
        hex => Rgb::from_hex(hex).ok_or_else(|| serde::de::Error::custom("expected a #RRGGBB color")),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridSettings {
    /// Show the dot grid on the desktop all the time (it always shows while editing).
    pub show_on_desktop: bool,
    #[serde(deserialize_with = "dot_color")]
    pub color: Rgb,
    /// Dot radius in device-independent pixels.
    pub size: f32,
    /// Opacity in percent.
    pub opacity: u8,
    /// Dots drift away from the mouse while editing.
    pub repel: bool,
}

impl Default for GridSettings {
    fn default() -> Self {
        GridSettings { show_on_desktop: false, color: WHITE, size: 1.5, opacity: 75, repel: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub zones: Vec<Zone>,
    pub show_zones: bool,
    /// Where boxes overlap, their colors mix (blue + red = purple).
    pub mix_colors: bool,
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
            mix_colors: true,
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
pub const DOT_COLORS: &[(&str, Rgb)] = &[
    ("White", WHITE),
    ("Light gray", Rgb(0xC0, 0xC0, 0xC0)),
    ("Dark gray", Rgb(0x55, 0x55, 0x55)),
    ("Black", Rgb(0x10, 0x10, 0x10)),
    ("Pastel green", PASTEL_GREEN),
    ("Sky", Rgb(0x8E, 0xC5, 0xFF)),
    ("Lavender", Rgb(0xC3, 0xB1, 0xE1)),
    ("Pink", Rgb(0xFF, 0xB3, 0xC7)),
    ("Lemon", Rgb(0xFD, 0xF2, 0x8C)),
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
    ("Transparent", 0),
    ("5%", 5),
    ("10%", 10),
    ("15%", 15),
    ("20%", 20),
    ("25%", 25),
    ("30%", 30),
    ("40%", 40),
    ("50%", 50),
    ("60%", 60),
    ("75%", 75),
    ("100%", 100),
];
pub const BORDER_OPACITIES: &[(&str, u8)] = &[
    ("10%", 10),
    ("25%", 25),
    ("50%", 50),
    ("75%", 75),
    ("95%", 95),
    ("100%", 100),
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
    ("Tiny", 1.0),
    ("Small", 1.5),
    ("Medium", 2.0),
    ("Large", 3.0),
];
pub const DOT_OPACITY: &[(&str, u8)] = &[
    ("10%", 10),
    ("25%", 25),
    ("40%", 40),
    ("50%", 50),
    ("60%", 60),
    ("75%", 75),
    ("90%", 90),
    ("100%", 100),
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
const OFF_CUSTOM_COLOR: u32 = 20;
const OFF_FILL: u32 = 30;
const OFF_BORDER: u32 = 50;
const OFF_BSTYLE: u32 = 60;
const OFF_BCOLOR_SAME: u32 = 70;
const OFF_BCOLOR: u32 = 71;
const OFF_BCOLOR_CUSTOM: u32 = 90;
const OFF_BOPACITY: u32 = 100;
const OFF_RADIUS: u32 = 110;
const OFF_POS_BORDER: u32 = 120;
const OFF_POS_INSIDE: u32 = 130;
const OFF_TSIZE: u32 = 140;
const OFF_TSTYLE: u32 = 150;
/// Style offsets are always below this value.
pub const STYLE_SPAN: u32 = 200;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StyleCmd {
    Color(Rgb),
    CustomColor,
    Fill(u8),
    BorderWidth(f32),
    BorderStyle(BorderStyle),
    BorderColor(Option<Rgb>),
    CustomBorderColor,
    BorderOpacity(u8),
    Radius(f32),
    TitlePos { on_border: bool, v: VAlign, h: HAlign },
    TitleSize(f32),
    TitleStyle(TitleStyle),
}

/// Every entry of the style submenus, with the current values checked.
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
    /// A disabled caption shown before this entry.
    pub heading: Option<&'static str>,
}

impl StyleCmd {
    pub fn decode(off: u32) -> Option<StyleCmd> {
        let i = |base: u32| (off - base) as usize;
        let within = |base: u32, len: usize| (base..base + len as u32).contains(&off);
        Some(match off {
            _ if within(OFF_COLOR, PALETTE.len()) => StyleCmd::Color(PALETTE[i(OFF_COLOR)].1),
            OFF_CUSTOM_COLOR => StyleCmd::CustomColor,
            _ if within(OFF_FILL, FILLS.len()) => StyleCmd::Fill(FILLS[i(OFF_FILL)].1),
            _ if within(OFF_BORDER, BORDER_WIDTHS.len()) => StyleCmd::BorderWidth(BORDER_WIDTHS[i(OFF_BORDER)].1),
            _ if within(OFF_BSTYLE, BORDER_STYLES.len()) => StyleCmd::BorderStyle(BORDER_STYLES[i(OFF_BSTYLE)].1),
            OFF_BCOLOR_SAME => StyleCmd::BorderColor(None),
            _ if within(OFF_BCOLOR, PALETTE.len()) => StyleCmd::BorderColor(Some(PALETTE[i(OFF_BCOLOR)].1)),
            OFF_BCOLOR_CUSTOM => StyleCmd::CustomBorderColor,
            _ if within(OFF_BOPACITY, BORDER_OPACITIES.len()) => {
                StyleCmd::BorderOpacity(BORDER_OPACITIES[i(OFF_BOPACITY)].1)
            }
            _ if within(OFF_RADIUS, RADII.len()) => StyleCmd::Radius(RADII[i(OFF_RADIUS)].1),
            _ if within(OFF_POS_BORDER, 6) => {
                let k = i(OFF_POS_BORDER);
                let v = if k < 3 { VAlign::Top } else { VAlign::Bottom };
                StyleCmd::TitlePos { on_border: true, v, h: HALIGNS[k % 3].1 }
            }
            _ if within(OFF_POS_INSIDE, 9) => {
                let k = i(OFF_POS_INSIDE);
                StyleCmd::TitlePos { on_border: false, v: VALIGNS[k / 3].1, h: HALIGNS[k % 3].1 }
            }
            _ if within(OFF_TSIZE, TITLE_SIZES.len()) => StyleCmd::TitleSize(TITLE_SIZES[i(OFF_TSIZE)].1),
            _ if within(OFF_TSTYLE, TITLE_STYLES.len()) => StyleCmd::TitleStyle(TITLE_STYLES[i(OFF_TSTYLE)].1),
            _ => return None,
        })
    }

    /// Commands that need a color picker before they can be applied.
    pub fn opens_dialog(self) -> bool {
        matches!(self, StyleCmd::CustomColor | StyleCmd::CustomBorderColor)
    }
}

impl ZoneStyle {
    pub fn apply(&mut self, cmd: StyleCmd) {
        match cmd {
            StyleCmd::Color(c) => self.color = c,
            StyleCmd::Fill(f) => self.fill = f,
            StyleCmd::BorderWidth(w) => self.border_width = w,
            StyleCmd::BorderStyle(s) => self.border_style = s,
            StyleCmd::BorderColor(c) => self.border_color = c,
            StyleCmd::BorderOpacity(o) => self.border_opacity = o,
            StyleCmd::Radius(r) => self.radius = r,
            StyleCmd::TitlePos { on_border, v, h } => {
                self.title_on_border = on_border;
                self.title_v = v;
                self.title_h = h;
            }
            StyleCmd::TitleSize(s) => self.title_size = s,
            StyleCmd::TitleStyle(s) => self.title_style = s,
            StyleCmd::CustomColor | StyleCmd::CustomBorderColor => {}
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
            heading: None,
        };
        let palette = |base: u32, current: Option<Rgb>| -> Vec<StyleEntry> {
            PALETTE
                .iter()
                .enumerate()
                .map(|(i, (name, c))| StyleEntry {
                    color: Some(*c),
                    ..entry(base + i as u32, (*name).into(), current == Some(*c))
                })
                .collect()
        };
        let is_custom = |c: Rgb| !PALETTE.iter().any(|(_, p)| *p == c);

        let mut colors = palette(OFF_COLOR, Some(self.color));
        colors.push(StyleEntry {
            separator: true,
            color: is_custom(self.color).then_some(self.color),
            ..entry(OFF_CUSTOM_COLOR, "Custom color…".into(), is_custom(self.color))
        });

        let fill = FILLS
            .iter()
            .enumerate()
            .map(|(i, (n, f))| entry(OFF_FILL + i as u32, (*n).into(), *f == self.fill))
            .collect();

        let mut border: Vec<StyleEntry> = BORDER_WIDTHS
            .iter()
            .enumerate()
            .map(|(i, (n, w))| entry(OFF_BORDER + i as u32, (*n).into(), (*w - self.border_width).abs() < 0.1))
            .collect();
        for (i, (n, s)) in BORDER_STYLES.iter().enumerate() {
            border.push(StyleEntry {
                separator: i == 0,
                ..entry(OFF_BSTYLE + i as u32, (*n).into(), *s == self.border_style)
            });
        }

        let mut bcolors = vec![StyleEntry {
            color: Some(self.color),
            ..entry(OFF_BCOLOR_SAME, "Same as box".into(), self.border_color.is_none())
        }];
        let mut pal = palette(OFF_BCOLOR, self.border_color);
        if let Some(first) = pal.first_mut() {
            first.separator = true;
        }
        bcolors.extend(pal);
        let custom_border = self.border_color.is_some_and(is_custom);
        bcolors.push(StyleEntry {
            separator: true,
            color: self.border_color.filter(|c| is_custom(*c)),
            ..entry(OFF_BCOLOR_CUSTOM, "Custom color…".into(), custom_border)
        });

        let bopacity = BORDER_OPACITIES
            .iter()
            .enumerate()
            .map(|(i, (n, o))| entry(OFF_BOPACITY + i as u32, (*n).into(), *o == self.border_opacity))
            .collect();

        let radius = RADII
            .iter()
            .enumerate()
            .map(|(i, (n, r))| entry(OFF_RADIUS + i as u32, (*n).into(), (*r - self.radius).abs() < 0.1))
            .collect();

        let mut pos = Vec::new();
        for k in 0..6usize {
            let v = if k < 3 { VAlign::Top } else { VAlign::Bottom };
            let (hn, h) = HALIGNS[k % 3];
            let vn = if k < 3 { "Top" } else { "Bottom" };
            pos.push(StyleEntry {
                heading: (k == 0).then_some("On the border line"),
                ..entry(
                    OFF_POS_BORDER + k as u32,
                    format!("{vn} {hn}"),
                    self.title_on_border && self.title_v == v && self.title_h == h,
                )
            });
        }
        for (vi, (vn, v)) in VALIGNS.iter().enumerate() {
            for (hi, (hn, h)) in HALIGNS.iter().enumerate() {
                let label = if *v == VAlign::Middle && *h == HAlign::Center {
                    "Center".to_string()
                } else {
                    format!("{vn} {hn}")
                };
                pos.push(StyleEntry {
                    heading: (vi == 0 && hi == 0).then_some("Inside the box"),
                    separator: vi == 0 && hi == 0,
                    ..entry(
                        OFF_POS_INSIDE + (vi * 3 + hi) as u32,
                        label,
                        !self.title_on_border && self.title_v == *v && self.title_h == *h,
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

        StyleMenu {
            sections: vec![
                ("Color", colors),
                ("Background opacity", fill),
                ("Border", border),
                ("Border color", bcolors),
                ("Border opacity", bopacity),
                ("Corner roundness", radius),
                ("Title position", pos),
                ("Title size and style", title),
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
        let mut seen = std::collections::HashSet::new();
        for (_, entries) in &menu.sections {
            for e in entries {
                assert!(e.offset < STYLE_SPAN);
                assert!(seen.insert(e.offset), "duplicate offset {}", e.offset);
                assert!(StyleCmd::decode(e.offset).is_some(), "offset {}", e.offset);
            }
        }
        assert_eq!(
            StyleCmd::decode(OFF_POS_INSIDE + 8),
            Some(StyleCmd::TitlePos { on_border: false, v: VAlign::Bottom, h: HAlign::Right })
        );
        assert_eq!(
            StyleCmd::decode(OFF_POS_BORDER + 1),
            Some(StyleCmd::TitlePos { on_border: true, v: VAlign::Top, h: HAlign::Center })
        );
    }

    #[test]
    fn default_title_is_top_center_on_the_line() {
        let s = ZoneStyle::default();
        assert!(s.title_on_border);
        assert_eq!((s.title_v, s.title_h), (VAlign::Top, HAlign::Center));
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
            style: ZoneStyle { border_color: Some(Rgb(1, 2, 3)), ..ZoneStyle::default() },
        });
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.zones, cfg.zones);
        // Unknown and missing fields are tolerated.
        let partial: Config = serde_json::from_str(r#"{"snap": false, "future": 1}"#).unwrap();
        assert!(!partial.snap);
        assert!(partial.show_zones);
    }

    #[test]
    fn reads_version_0_1_dot_colors() {
        let cfg: Config = serde_json::from_str(r#"{"grid": {"color": "black", "show_on_desktop": true}}"#).unwrap();
        assert_eq!(cfg.grid.color, Rgb(0x10, 0x10, 0x10));
        assert!(cfg.grid.show_on_desktop);
        assert!(cfg.grid.repel);
    }
}
