//! The tray icon and its menu (StatusNotifierItem over D-Bus: shown by KDE Plasma, Xfce,
//! Cinnamon, MATE and Budgie, and by GNOME with the AppIndicator extension Ubuntu ships).

use super::{fonts, UserEvent, VERSION};
use crate::commands as cmd;
use crate::model::{self, GridSettings, Rgb, ZoneStyle};
use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::MenuItem;
use winit::event_loop::EventLoopProxy;

/// What the menu shows (a copy of the app state, refreshed after every change).
#[derive(Clone)]
pub struct View {
    pub editing: bool,
    pub show_zones: bool,
    pub mix_colors: bool,
    pub snap: bool,
    pub grid: GridSettings,
    pub defaults: ZoneStyle,
    pub font: String,
    pub autostart: bool,
    pub installed: bool,
    pub can_pick_color: bool,
    pub any_zones: bool,
}

pub struct DeskTray {
    pub view: View,
    proxy: EventLoopProxy<UserEvent>,
    icons: Vec<ksni::Icon>,
}

type Item = MenuItem<DeskTray>;

impl DeskTray {
    pub fn new(view: View, proxy: EventLoopProxy<UserEvent>) -> DeskTray {
        let icons = [22, 32, 48, 64].into_iter().filter_map(icon).collect();
        DeskTray { view, proxy, icons }
    }

    fn send(&self, id: u32) {
        let _ = self.proxy.send_event(UserEvent::Command(id));
    }
}

fn icon(size: u32) -> Option<ksni::Icon> {
    let pm = crate::icon::render(size, crate::icon::WHITE, crate::icon::PASTEL_GREEN, Some(crate::icon::DARK))?;
    // ARGB32, network byte order, straight alpha.
    let data = pm.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.alpha(), c.red(), c.green(), c.blue()]
    });
    Some(ksni::Icon { width: size as i32, height: size as i32, data: data.collect() })
}

/// A small color square as PNG (tray menu items take PNG icons).
fn swatch(c: Rgb) -> Vec<u8> {
    let size = 16u32;
    let Some(mut pm) = tiny_skia::Pixmap::new(size, size) else { return Vec::new() };
    let painter = crate::render::Painter { swap_rb: false };
    if let Some(p) = crate::render::rounded_rect(crate::geom::Rect::new(1.5, 1.5, 13.0, 13.0), 3.0) {
        pm.fill_path(&p, &painter.paint(c, 1.0), tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
        let stroke = tiny_skia::Stroke { width: 1.0, ..Default::default() };
        pm.stroke_path(&p, &painter.paint(Rgb(0x55, 0x55, 0x55), 0.9), &stroke, tiny_skia::Transform::identity(), None);
    }
    let rgba: Vec<u8> = pm.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }).collect();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let Ok(mut w) = enc.write_header() else { return Vec::new() };
        if w.write_image_data(&rgba).is_err() {
            return Vec::new();
        }
    }
    out
}

fn action(label: &str, id: u32) -> Item {
    action_enabled(label, id, true)
}

fn action_enabled(label: &str, id: u32, enabled: bool) -> Item {
    StandardItem { label: label.into(), enabled, activate: Box::new(move |t: &mut DeskTray| t.send(id)), ..Default::default() }.into()
}

fn check(label: &str, id: u32, on: bool) -> Item {
    CheckmarkItem { label: label.into(), checked: on, activate: Box::new(move |t: &mut DeskTray| t.send(id)), ..Default::default() }.into()
}

fn heading(label: &str) -> Item {
    StandardItem { label: label.into(), enabled: false, ..Default::default() }.into()
}

fn sub(label: &str, submenu: Vec<Item>) -> Item {
    SubMenu { label: label.into(), submenu, ..Default::default() }.into()
}

/// A group of choices where one is selected (`ids[i]` is sent for option `i`).
fn radios(options: Vec<(String, Option<Rgb>, u32)>, selected: Option<usize>) -> Item {
    let ids: Vec<u32> = options.iter().map(|o| o.2).collect();
    RadioGroup {
        selected: selected.unwrap_or(usize::MAX),
        select: Box::new(move |t: &mut DeskTray, i: usize| {
            if let Some(id) = ids.get(i) {
                t.send(*id);
            }
        }),
        options: options
            .into_iter()
            .map(|(label, color, _)| RadioItem { label, icon_data: color.map(swatch).unwrap_or_default(), ..Default::default() })
            .collect(),
        ..Default::default()
    }
    .into()
}

/// The Color / Background opacity / ... submenus for a style (ids `base + offset`).
fn style_menus(style: &ZoneStyle, base: u32, can_pick_color: bool) -> Vec<Item> {
    let mut out = Vec::new();
    for (title, entries) in style.menu().sections {
        let mut items: Vec<Item> = Vec::new();
        let mut group: Vec<(String, Option<Rgb>, u32)> = Vec::new();
        let mut selected = None;
        let flush = |items: &mut Vec<Item>, group: &mut Vec<(String, Option<Rgb>, u32)>, selected: &mut Option<usize>| {
            if !group.is_empty() {
                items.push(radios(std::mem::take(group), selected.take()));
            }
        };
        for e in entries {
            if e.separator {
                flush(&mut items, &mut group, &mut selected);
                items.push(MenuItem::Separator);
            }
            if let Some(h) = e.heading {
                flush(&mut items, &mut group, &mut selected);
                items.push(heading(h));
            }
            let custom = model::StyleCmd::decode(e.offset).is_some_and(|c| c.opens_dialog());
            if custom && !can_pick_color {
                continue;
            }
            if e.checked {
                selected = Some(group.len());
            }
            group.push((e.label.clone(), e.color, base + e.offset));
        }
        flush(&mut items, &mut group, &mut selected);
        out.push(sub(title, items));
    }
    out
}

impl ksni::Tray for DeskTray {
    fn id(&self) -> String {
        "deskchalk".into()
    }

    fn title(&self) -> String {
        "DeskChalk".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let description = if self.view.editing { "Editing boxes (Esc to finish)" } else { "Click to draw boxes" };
        ksni::ToolTip { title: "DeskChalk".into(), description: description.into(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(cmd::EDIT);
    }

    fn menu(&self) -> Vec<Item> {
        let v = &self.view;
        let g = &v.grid;
        let mut m: Vec<Item> = vec![
            action(if v.editing { "Done editing" } else { "Draw / edit boxes" }, cmd::EDIT),
            check("Show boxes", cmd::SHOW_ZONES, v.show_zones),
            check("Mix colors where boxes overlap", cmd::MIX_COLORS, v.mix_colors),
            MenuItem::Separator,
            check("Grid view (dots on the wallpaper)", cmd::GRID_SHOW, g.show_on_desktop),
            check("Snap boxes to the grid", cmd::SNAP, v.snap),
        ];

        let mut dots = vec![
            check("Dots move away from the mouse (while drawing)", cmd::GRID_REPEL, g.repel),
            MenuItem::Separator,
            heading("Spacing"),
        ];
        let spacing = model::GRID_SPACINGS.iter().position(|(_, s)| (s - g.spacing).abs() < 0.5);
        dots.push(radios(
            model::GRID_SPACINGS.iter().enumerate().map(|(i, (n, _))| (n.to_string(), None, cmd::GRID_SPACING + i as u32)).collect(),
            spacing,
        ));
        dots.push(MenuItem::Separator);
        dots.push(heading("Color"));
        let mut colors: Vec<(String, Option<Rgb>, u32)> =
            model::DOT_COLORS.iter().enumerate().map(|(i, (n, c))| (n.to_string(), Some(*c), cmd::DOT_COLOR + i as u32)).collect();
        let mut color_sel = model::DOT_COLORS.iter().position(|(_, c)| *c == g.color);
        if v.can_pick_color {
            if color_sel.is_none() {
                color_sel = Some(colors.len());
            }
            colors.push(("Custom color…".into(), color_sel.filter(|&i| i == colors.len()).map(|_| g.color), cmd::DOT_CUSTOM));
        }
        dots.push(radios(colors, color_sel));
        dots.push(MenuItem::Separator);
        dots.push(heading("Size"));
        dots.push(radios(
            model::DOT_SIZES.iter().enumerate().map(|(i, (n, _))| (n.to_string(), None, cmd::GRID_SIZE + i as u32)).collect(),
            model::DOT_SIZES.iter().position(|(_, s)| (s - g.size).abs() < 0.01),
        ));
        dots.push(MenuItem::Separator);
        dots.push(heading("Opacity"));
        dots.push(radios(
            model::DOT_OPACITY.iter().enumerate().map(|(i, (n, _))| (n.to_string(), None, cmd::GRID_OPACITY + i as u32)).collect(),
            model::DOT_OPACITY.iter().position(|(_, o)| *o == g.opacity),
        ));
        m.push(sub("Grid dots", dots));
        m.push(MenuItem::Separator);

        let mut style = style_menus(&v.defaults, cmd::STYLE_DEFAULT, v.can_pick_color);
        style.push(MenuItem::Separator);
        style.push(action_enabled("Apply this style to all boxes", cmd::APPLY_ALL, v.any_zones));
        m.push(sub("New box style", style));

        let current = fonts::choice_key(&v.font);
        m.push(sub(
            "Title font",
            vec![radios(
                fonts::FONT_CHOICES.iter().enumerate().map(|(i, f)| (f.label.to_string(), None, cmd::FONT + i as u32)).collect(),
                fonts::FONT_CHOICES.iter().position(|f| f.key == current),
            )],
        ));
        m.push(MenuItem::Separator);
        m.push(check("Start at login", cmd::AUTOSTART, v.autostart));
        m.push(MenuItem::Separator);
        m.push(action("Support DeskChalk on Ko-fi", cmd::SUPPORT));
        m.push(action(&format!("About DeskChalk {VERSION}"), cmd::ABOUT));
        m.push(action(if v.installed { "Uninstall DeskChalk…" } else { "Remove DeskChalk (portable)…" }, cmd::UNINSTALL));
        m.push(action("Exit", cmd::EXIT));
        m
    }
}
