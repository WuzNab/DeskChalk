//! Menu command ids, shared by the Windows and Linux menus.
#![allow(dead_code)]

pub const EDIT: u32 = 1;
pub const SHOW_ZONES: u32 = 2;
pub const GRID_SHOW: u32 = 3;
pub const SNAP: u32 = 4;
pub const AUTOSTART: u32 = 5;
pub const MINIMIZE: u32 = 6;
pub const HOTKEY: u32 = 7;
pub const SUPPORT: u32 = 8;
pub const ABOUT: u32 = 9;
pub const UNINSTALL: u32 = 10;
pub const EXIT: u32 = 11;
pub const AUTO_ARRANGE: u32 = 12;
pub const ALIGN_GRID: u32 = 13;
pub const RECOVER: u32 = 14;
pub const REFRESH: u32 = 15;
pub const UNDO: u32 = 16;
pub const REDO: u32 = 17;
pub const DELETE_ALL: u32 = 18;
pub const APPLY_ALL: u32 = 19;
pub const MIX_COLORS: u32 = 22;
pub const GRID_REPEL: u32 = 23;
pub const DOT_CUSTOM: u32 = 24;
pub const Z_RENAME: u32 = 30;
pub const Z_DUPLICATE: u32 = 31;
pub const Z_FRONT: u32 = 32;
pub const Z_BACK: u32 = 33;
pub const Z_DELETE: u32 = 34;
pub const Z_MAKE_DEFAULT: u32 = 35;
pub const Z_RESET_STYLE: u32 = 36;
/// + index into model::DOT_SIZES
pub const GRID_SIZE: u32 = 100;
/// + index into model::DOT_OPACITY
pub const GRID_OPACITY: u32 = 120;
/// + index into model::ICON_SIZES
pub const ICON_SIZE: u32 = 140;
/// + index into fonts::FONT_CHOICES
pub const FONT: u32 = 160;
/// + index into model::DOT_COLORS
pub const DOT_COLOR: u32 = 180;
/// + style offset, applies to the clicked box
pub const STYLE_ZONE: u32 = 1000;
/// + style offset, applies to the style for new boxes
pub const STYLE_DEFAULT: u32 = 2000;
/// + index into model::GRID_SPACINGS (Linux: the dot grid has no icon grid to follow)
pub const GRID_SPACING: u32 = 200;
