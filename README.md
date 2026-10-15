<p align="center">
  <img src="assets/deskchalk.svg" width="112" alt="DeskChalk logo: a white pencil drawing a pastel green line">
</p>

<h1 align="center">DeskChalk</h1>

<p align="center">
  Draw labeled boxes on your desktop wallpaper, behind your icons.<br>
  A tiny tray app for sorting desktop chaos.
</p>

<p align="center">
  <a href="https://github.com/WuzNab/DeskChalk/releases"><img alt="Download for Windows" src="https://img.shields.io/badge/download-for%20Windows-77DD77"></a>
  <a href="https://ko-fi.com/wuznab"><img alt="Support me on Ko-fi" src="https://ko-fi.com/img/githubbutton_sm.svg" height="20"></a>
</p>

<p align="center">
  <img src="assets/demo.gif" width="800" alt="Drawing boxes on the desktop, naming them, styling them from the right-click menu, and dragging icons into them">
</p>

---

You dump files on your desktop while you work. Folders are too slow. DeskChalk lets you
draw labeled zones on the desktop background, on the fly, and drag icons into them.

- **Boxes are just drawings.** They never hold, move, or touch your files. Icons stay normal desktop icons.
- **Behind your icons.** Boxes live between the wallpaper and the icons, so everything still clicks and drags as usual.
- **Snaps to the icon grid.** A dot grid matches your desktop icon spacing, so a box holds exactly *n × m* icons. Free-form works too.
- **Titles on the line.** Titles sit in a gap on the box's border by default, so icons never cover them. Six positions on the line, nine inside.
- **Colors mix.** Where boxes overlap, their colors blend (blue and red make purple) without getting darker.
- **Make it yours.** Box color and background opacity, border color, opacity, width and style, corner roundness, title font, size and style, and dot color, size and opacity.
- **Per monitor.** Each box belongs to the screen you drew it on and stays there, even after resolution or scaling changes.
- **Lightweight.** A ~1 MB app that uses almost no memory or CPU while idle. No background services, no hooks.

<p align="center">
  <img src="assets/preview.png" width="800" alt="Example boxes: titles on the border line, a label chip, a dashed border, overlapping colors mixing, and the dot grid">
</p>

## Install

1. Download **DeskChalk_Setup_v*x.y.z*.exe** from [Releases](https://github.com/WuzNab/DeskChalk/releases) (the newest one is at the top).
2. Run it and follow the wizard. No admin rights needed; it installs just for you. You can choose to:
   - start DeskChalk when Windows starts (on by default)
   - add it to the Start menu
   - create a desktop shortcut
3. Find the pencil icon in your tray (on Windows 11 it may be under the **^** overflow; drag it onto the taskbar to keep it visible).

Closed DeskChalk? Start it again from the Start menu or the desktop shortcut. If it's already running, opening it
shows its menu right where you clicked.

DeskChalk isn't code-signed yet, so Windows SmartScreen may say *"Windows protected your PC"*.
Click **More info**, then **Run anyway**.

**Updating:** run the newer `DeskChalk_Setup_v….exe`. It closes DeskChalk, updates it in place and keeps your boxes.
Choose **Clean reinstall** on the first page if you'd rather start fresh (this deletes all boxes and settings).

**Portable:** prefer not to install? Download **DeskChalk_Portable_v*x.y.z*.exe** and run it from anywhere.

## Use it

**Double-click the tray icon** (or press **Ctrl+Alt+D**) to start drawing. Your windows minimize, a dot grid appears, and you can:

| Do this | To |
| --- | --- |
| Drag on empty space | Draw a box (you'll be asked for a title) |
| Drag a box / its edges | Move / resize it |
| Double-click a box, or **F2** | Rename it |
| Right-click a box | Color, background opacity, border, border color and opacity, corner roundness, title position, size and style, duplicate, delete |
| Hold **Alt** while dragging | Toggle snapping (free-form or grid) |
| **Arrow keys** | Nudge the selected box (one icon slot; in free-form 1 px, or 10 px with Shift) |
| **Ctrl+Z** / **Ctrl+Y** | Undo / redo |
| **Ctrl+D** / **Del** | Duplicate / delete the selected box |
| **Esc** | Done; your windows come back |

Then drag your icons into the boxes like you normally would.

Everything else lives in the **tray menu**: show or hide boxes, color mixing, grid view (dots on the desktop), dot color, size, opacity
and the dots-move-away-from-the-mouse effect, the default style for new boxes, title font, desktop icon size, auto arrange and
align to grid, start with Windows, and uninstall. Menus stay open while you change settings, so you can adjust several in a row.

> **Tip:** turn **off** *Auto arrange icons* (DeskChalk offers to do this the first time, or use
> *Desktop icons* in the tray menu). Otherwise Windows pulls your icons back into a column.
> Leaving *Align icons to grid* on keeps icons neatly inside snapped boxes.

## Uninstall

Tray menu → **Uninstall DeskChalk…**, **Settings → Apps → Installed apps → DeskChalk**, or **Control Panel → Programs and Features**.
This removes the app, its shortcuts, its startup entry, and your boxes and settings. Nothing is left behind,
and your desktop icons are not touched.

The portable version: tray menu → **Remove DeskChalk (portable)…** deletes its boxes, settings and startup entry;
then delete the portable exe yourself.

## Requirements

- Windows 10 or 11, 64-bit. Developed and tested on Windows 11 25H2; the Windows 10 code path is implemented but less tested.

Linux support planned.

## Troubleshooting

- **Boxes don't show up:** tray menu → *Desktop icons* → **Refresh desktop layer**.
- **Something looks off:** run `DeskChalk.exe --diag` from a terminal and include the output in an issue.
  Start with `--log` to write a log to `%APPDATA%\DeskChalk\deskchalk.log`.
- **Restarting Explorer:** DeskChalk reattaches by itself within a couple of seconds. Restart Explorer from
  Task Manager (*Windows Explorer → Restart*) rather than killing `explorer.exe`; a hard kill can lose your icon positions.
- **Settings file:** `%APPDATA%\DeskChalk\config.json` (per user). If it ever becomes unreadable, DeskChalk keeps
  a copy as `config.broken.json` and starts fresh.

<details>
<summary>Command-line options</summary>

| Option | Effect |
| --- | --- |
| `--edit` | Start in drawing mode (or switch the running DeskChalk into it) |
| `--quit` | Close the running DeskChalk |
| `--diag` | Print what DeskChalk detects (desktop windows, monitors, icon grid) |
| `--log` | Write a log file |

The setup supports the usual Inno Setup switches, e.g. `DeskChalk_Setup_v0.2.2.exe /SILENT` for an unattended install.

</details>

## Build from source

Install [Rust](https://rustup.rs) (MSVC toolchain; Visual Studio Build Tools provide the linker), then:

```
cargo build --release
```

The app is `target\release\DeskChalk.exe` and runs from there without installing. To build the setup wizard,
install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and run `iscc /DAppVersion=0.2.2 installer\DeskChalk.iss`.
Push a `v*` tag to have GitHub Actions build both and publish a release.

## Support

DeskChalk is free. If it tidies up your desktop, you can [buy me a coffee on Ko-fi](https://ko-fi.com/wuznab) ☕

## License

[PolyForm Strict 1.0.0](LICENSE.txt): free for personal and other noncommercial use. Redistributing DeskChalk
or publishing modified versions isn't permitted.

The icon is based on [Material Design Icons](https://pictogrammers.com/library/mdi/icon/draw/) "draw"
(Apache License 2.0).
