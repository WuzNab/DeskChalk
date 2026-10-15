//! Desktop notifications (the freedesktop D-Bus service) and small dialogs through zenity
//! or kdialog, whichever the desktop has.

use super::util;
use crate::model::Rgb;
use std::collections::HashMap;

/// Shows a notification without waiting for the notification service.
pub fn notify(summary: &str, body: &str) {
    let (summary, body) = (summary.to_string(), body.to_string());
    std::thread::spawn(move || notify_now(&summary, &body));
}

/// Shows a notification and waits until the service took it (used right before exiting).
pub fn notify_now(summary: &str, body: &str) {
    let Ok(conn) = zbus::blocking::Connection::session() else { return };
    let hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    let r = conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("DeskChalk", 0u32, "deskchalk", summary, body, Vec::<&str>::new(), hints, 8000i32),
    );
    if let Err(e) = r {
        log!("notification failed: {e}");
    }
}

/// True if the desktop shows tray icons (a StatusNotifierWatcher is running).
pub fn tray_available() -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else { return false };
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &("org.kde.StatusNotifierWatcher",),
    )
    .ok()
    .and_then(|m| m.body().deserialize::<bool>().ok())
    .unwrap_or(false)
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Zenity,
    Kdialog,
}

fn tool() -> Option<Tool> {
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("KDE"));
    let order = if kde { [Tool::Kdialog, Tool::Zenity] } else { [Tool::Zenity, Tool::Kdialog] };
    order.into_iter().find(|t| util::have(if *t == Tool::Zenity { "zenity" } else { "kdialog" }))
}

/// Asks a yes/no question. None if there is no dialog program to ask with.
pub fn ask(question: &str) -> Option<bool> {
    Some(match tool()? {
        Tool::Zenity => {
            // zenity reads the text as Pango markup.
            let text = question.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            util::run("zenity", &["--question", "--title=DeskChalk", "--width=420", &format!("--text={text}")])
        }
        Tool::Kdialog => util::run("kdialog", &["--title", "DeskChalk", "--yesno", question]),
    })
}

pub fn can_pick_color() -> bool {
    tool().is_some()
}

/// Opens a color picker. None if cancelled or there is no dialog program.
pub fn choose_color(initial: Rgb) -> Option<Rgb> {
    let hex = initial.to_hex();
    let out = match tool()? {
        Tool::Zenity => util::output("zenity", &["--color-selection", "--title=DeskChalk", &format!("--color={hex}")])?,
        Tool::Kdialog => util::output("kdialog", &["--title", "DeskChalk", "--getcolor", "--default", &hex])?,
    };
    parse_color(&out)
}

/// "#rrggbb", "#rrrrggggbbbb", "rgb(r,g,b)" or "rgba(r,g,b,a)".
fn parse_color(s: &str) -> Option<Rgb> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        return match h.len() {
            6 => Rgb::from_hex(s),
            12 => {
                let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
                Some(Rgb(c(0)?, c(4)?, c(8)?))
            }
            _ => None,
        };
    }
    let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
    let v: Vec<u8> = inner.split(',').take(3).filter_map(|p| p.trim().parse::<f32>().ok()).map(|f| f.round().clamp(0.0, 255.0) as u8).collect();
    (v.len() == 3).then(|| Rgb(v[0], v[1], v[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_from_dialogs() {
        assert_eq!(parse_color("rgb(255,0,10)"), Some(Rgb(255, 0, 10)));
        assert_eq!(parse_color("rgba(1,2,3,0.5)\n"), Some(Rgb(1, 2, 3)));
        assert_eq!(parse_color("#77dd77"), Some(Rgb(0x77, 0xDD, 0x77)));
        assert_eq!(parse_color("#ffff00000000"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("nope"), None);
    }
}
