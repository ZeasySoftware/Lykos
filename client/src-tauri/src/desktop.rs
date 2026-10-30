//! Desktop-Fenster (nur Windows/macOS/Linux): Schließen in den Infobereich, Tray-Symbol mit Menü und
//! Ungelesen-Punkt, Einzelinstanz, „Mit Windows starten“ (minimiert) und Snap-Layouts für die eigene
//! Titelleiste.
//!
//! - **Schließen:** `CloseRequested` des Hauptfensters richtet sich nach dem Modus, den die Oberfläche per
//!   [`desktop_set_close_mode`] meldet: `tray` versteckt das Fenster (auch bis die Oberfläche geladen ist),
//!   `quit` schließt wirklich, `ask` schickt `close-requested` (Nutzlast: laufende Nummer) an die Oberfläche,
//!   die nachfragt und per [`desktop_close_resolve`] antwortet. Bestätigt sie das Ereignis nicht binnen
//!   [`CLOSE_ACK_TIMEOUT`] ([`desktop_close_ack`], z. B. weil die Seite gerade neu lädt), gilt wieder das
//!   bisherige Verhalten: verstecken. Ein laufender Sprachanruf bleibt im Tray aktiv – die Oberfläche läuft
//!   unsichtbar weiter. „Beenden“ im Tray-Menü beendet wirklich (`AppHandle::exit`), ebenso der Updater
//!   (Windows: Plugin beendet den Prozess selbst, sonst `restart`) – beides geht nicht über `CloseRequested`.
//! - **Tray-Menü:** Öffnen, Mikrofon stumm, Taub, Nach Updates suchen, Beenden. Stumm/Taub/Updates schickt
//!   das Ereignis `tray-action` an die Oberfläche (dort liegt der Voice-Zustand); die Oberfläche meldet den
//!   tatsächlichen Zustand per [`desktop_tray_update`] zurück (Haken, Punkt, Tooltip).
//! - **Snap-Layouts:** Ohne Fensterrahmen zeigt Windows 11 das Snap-Menü nicht beim Überfahren des eigenen
//!   Maximieren-Knopfs (die WebView liegt über dem Nicht-Client-Bereich). [`desktop_snap_overlay`] öffnet es
//!   stattdessen mit Win+Z – derselbe Weg wie `tauri-plugin-decorum`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, Window, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;

/// Startargument des Autostarts: Fenster nicht zeigen, nur ins Tray.
pub const MINIMIZED_ARG: &str = "--minimized";
const MAIN_WINDOW: &str = "main";
const TRAY_ID: &str = "main";
/// Ereignis an die Oberfläche (Nutzlast: `toggle-mute`, `toggle-deaf`, `check-updates`).
const TRAY_EVENT: &str = "tray-action";
/// Ereignis an die Oberfläche: Nachfrage beim Schließen (Nutzlast: laufende Nummer für die Bestätigung).
const CLOSE_EVENT: &str = "close-requested";
/// So lange darf die Oberfläche brauchen, um die Nachfrage zu bestätigen – sonst wird versteckt.
const CLOSE_ACK_TIMEOUT: Duration = Duration::from_millis(1500);
const ICON_PNG: &[u8] = include_bytes!("../icons/64x64.png");

/// Punkt am Tray-Symbol.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
  None,
  /// Ungelesenes in Communities (heller Punkt).
  Unread,
  /// Erwähnungen oder ungelesene Direktnachrichten (roter Punkt).
  Mention,
}

impl Badge {
  fn parse(value: &str) -> Self {
    match value {
      "mention" => Badge::Mention,
      "unread" => Badge::Unread,
      _ => Badge::None,
    }
  }

  fn color(self) -> Option<[u8; 3]> {
    match self {
      Badge::None => None,
      // Wie `--text-strong` bzw. `--danger` in tokens.css.
      Badge::Unread => Some([0xee, 0xf0, 0xf5]),
      Badge::Mention => Some([0xf0, 0x71, 0x6b]),
    }
  }
}

/// Verhalten beim Schließen des Hauptfensters (siehe Moduldoku).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum CloseMode {
  Tray = 0,
  Quit = 1,
  Ask = 2,
}

impl CloseMode {
  fn parse(s: &str) -> Option<Self> {
    match s {
      "tray" => Some(Self::Tray),
      "quit" => Some(Self::Quit),
      "ask" => Some(Self::Ask),
      _ => None,
    }
  }

  fn from_u8(v: u8) -> Self {
    match v {
      1 => Self::Quit,
      2 => Self::Ask,
      _ => Self::Tray,
    }
  }
}

struct DesktopState {
  close_mode: AtomicU8,
  /// Laufende Nummer der letzten Nachfrage bzw. der zuletzt bestätigten.
  close_seq: AtomicU64,
  close_acked: AtomicU64,
  mute: CheckMenuItem<Wry>,
  deaf: CheckMenuItem<Wry>,
  badge: Mutex<Badge>,
  /// RGBA des App-Symbols (ohne Punkt) und Kantenlänge.
  icon: (Vec<u8>, u32, u32),
}

/// Holt das Hauptfenster zurück (aus dem Tray, minimiert oder hinter anderen Fenstern).
pub fn show_main(app: &AppHandle) {
  if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
  }
}

/// Wurde die App per Autostart (minimiert) gestartet?
pub fn started_minimized<I: IntoIterator<Item = S>, S: AsRef<str>>(args: I) -> bool {
  args.into_iter().any(|a| a.as_ref() == MINIMIZED_ARG)
}

/// Zweiter Start (Einzelinstanz): vorhandenes Fenster nach vorn holen – außer beim Autostart.
pub fn on_second_instance(app: &AppHandle, args: Vec<String>, _cwd: String) {
  if !started_minimized(&args) {
    show_main(app);
  }
}

/// Tray-Symbol und -Menü anlegen; Fenster zeigen, wenn nicht minimiert gestartet.
pub fn setup(app: &mut App) -> tauri::Result<()> {
  let open = MenuItem::with_id(app, "open", "Lykos öffnen", true, None::<&str>)?;
  let mute = CheckMenuItem::with_id(app, "mute", "Mikrofon stumm", true, false, None::<&str>)?;
  let deaf = CheckMenuItem::with_id(app, "deaf", "Taub", true, false, None::<&str>)?;
  let updates = MenuItem::with_id(app, "updates", "Nach Updates suchen", true, None::<&str>)?;
  let quit = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?;
  let menu = Menu::with_items(
    app,
    &[
      &open,
      &PredefinedMenuItem::separator(app)?,
      &mute,
      &deaf,
      &PredefinedMenuItem::separator(app)?,
      &updates,
      &PredefinedMenuItem::separator(app)?,
      &quit,
    ],
  )?;

  let base = Image::from_bytes(ICON_PNG)?;
  let icon = (base.rgba().to_vec(), base.width(), base.height());

  TrayIconBuilder::with_id(TRAY_ID)
    .icon(base.to_owned())
    .tooltip("Lykos")
    .menu(&menu)
    .show_menu_on_left_click(false)
    .on_menu_event(|app, event| match event.id().as_ref() {
      "open" => show_main(app),
      "mute" => {
        let _ = app.emit(TRAY_EVENT, "toggle-mute");
      }
      "deaf" => {
        let _ = app.emit(TRAY_EVENT, "toggle-deaf");
      }
      "updates" => {
        show_main(app);
        let _ = app.emit(TRAY_EVENT, "check-updates");
      }
      "quit" => app.exit(0),
      _ => {}
    })
    .on_tray_icon_event(|tray, event| {
      if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
        show_main(tray.app_handle());
      }
    })
    .build(app)?;

  app.manage(DesktopState {
    close_mode: AtomicU8::new(CloseMode::Tray as u8),
    close_seq: AtomicU64::new(0),
    close_acked: AtomicU64::new(0),
    mute,
    deaf,
    badge: Mutex::new(Badge::None),
    icon,
  });

  // Das Fenster startet unsichtbar (`visible: false` in tauri.conf.json), damit der Autostart nichts aufblitzen lässt.
  if !started_minimized(std::env::args())
    && let Some(window) = app.get_webview_window(MAIN_WINDOW)
  {
    window.show()?;
    window.set_focus()?;
  }
  Ok(())
}

/// `on_window_event`: Schließen des Hauptfensters → verstecken, beenden oder nachfragen (siehe Moduldoku).
pub fn on_window_event(window: &Window, event: &WindowEvent) {
  let WindowEvent::CloseRequested { api, .. } = event else { return };
  if window.label() != MAIN_WINDOW {
    return;
  }
  let Some(state) = window.try_state::<DesktopState>() else { return };
  match CloseMode::from_u8(state.close_mode.load(Ordering::Relaxed)) {
    CloseMode::Quit => {}
    CloseMode::Tray => {
      api.prevent_close();
      let _ = window.hide();
    }
    CloseMode::Ask => {
      api.prevent_close();
      let seq = state.close_seq.fetch_add(1, Ordering::Relaxed) + 1;
      if window.emit(CLOSE_EVENT, seq).is_err() {
        let _ = window.hide();
        return;
      }
      // Antwortet die Oberfläche nicht (lädt gerade, hängt), nicht stehen bleiben: wie bisher verstecken.
      let window = window.clone();
      std::thread::spawn(move || {
        std::thread::sleep(CLOSE_ACK_TIMEOUT);
        let acked = window
          .try_state::<DesktopState>()
          .is_some_and(|s| s.close_acked.load(Ordering::Relaxed) >= seq);
        if !acked {
          let _ = window.hide();
        }
      });
    }
  }
}

/// Verhalten beim Schließen übernehmen: `ask`, `tray` oder `quit` (Einstellungen „Desktop-App“).
#[tauri::command]
pub fn desktop_set_close_mode(app: AppHandle, mode: String) -> Result<(), String> {
  let mode = CloseMode::parse(&mode).ok_or_else(|| format!("unbekannter Modus: {mode}"))?;
  if let Some(state) = app.try_state::<DesktopState>() {
    state.close_mode.store(mode as u8, Ordering::Relaxed);
  }
  Ok(())
}

/// Die Oberfläche hat die Nachfrage `seq` erhalten und zeigt den Dialog.
#[tauri::command]
pub fn desktop_close_ack(app: AppHandle, seq: u64) {
  if let Some(state) = app.try_state::<DesktopState>() {
    state.close_acked.fetch_max(seq, Ordering::Relaxed);
  }
}

/// Antwort auf die Nachfrage: `tray` versteckt das Fenster, `quit` beendet Lykos.
#[tauri::command]
pub fn desktop_close_resolve(app: AppHandle, action: String) -> Result<(), String> {
  match CloseMode::parse(&action) {
    Some(CloseMode::Tray) => {
      if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        window.hide().map_err(|e| e.to_string())?;
      }
      Ok(())
    }
    Some(CloseMode::Quit) => {
      app.exit(0);
      Ok(())
    }
    _ => Err(format!("unbekannte Aktion: {action}")),
  }
}

/// Tray an den Zustand der Oberfläche angleichen: Haken bei Stumm/Taub, Punkt am Symbol, Tooltip.
#[tauri::command]
pub fn desktop_tray_update(
  app: AppHandle,
  muted: bool,
  deafened: bool,
  badge: String,
  tooltip: String,
) -> Result<(), String> {
  let Some(state) = app.try_state::<DesktopState>() else { return Ok(()) };
  state.mute.set_checked(muted).map_err(|e| e.to_string())?;
  state.deaf.set_checked(deafened).map_err(|e| e.to_string())?;
  let Some(tray) = app.tray_by_id(TRAY_ID) else { return Ok(()) };
  tray.set_tooltip(Some(tooltip)).map_err(|e| e.to_string())?;

  let badge = Badge::parse(&badge);
  let mut current = state.badge.lock().map_err(|e| e.to_string())?;
  if *current != badge {
    let (rgba, width, height) = &state.icon;
    let mut pixels = rgba.clone();
    if let Some(color) = badge.color() {
      draw_badge(&mut pixels, *width, *height, color);
    }
    tray
      .set_icon(Some(Image::new_owned(pixels, *width, *height)))
      .map_err(|e| e.to_string())?;
    *current = badge;
  }
  Ok(())
}

/// Autostart eingerichtet?
#[tauri::command]
pub fn desktop_autostart_get(app: AppHandle) -> Result<bool, String> {
  app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// „Mit Windows starten“ ein-/ausschalten (startet minimiert im Infobereich).
#[tauri::command]
pub fn desktop_autostart_set(app: AppHandle, enabled: bool) -> Result<(), String> {
  let launch = app.autolaunch();
  if enabled { launch.enable() } else { launch.disable() }.map_err(|e| e.to_string())
}

/// Snap-Layouts von Windows 11 öffnen (Hover über dem eigenen Maximieren-Knopf).
#[tauri::command]
pub fn desktop_snap_overlay(window: Window) {
  #[cfg(windows)]
  {
    // Win+Z gilt dem Vordergrundfenster.
    if !window.is_focused().unwrap_or(false) {
      let _ = window.set_focus();
    }
    win_z();
  }
  #[cfg(not(windows))]
  let _ = window;
}

#[cfg(windows)]
fn win_z() {
  use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_LWIN,
  };
  let key = |vk: u16, up: bool| INPUT {
    r#type: INPUT_KEYBOARD,
    Anonymous: INPUT_0 {
      ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
    },
  };
  let z = u16::from(b'Z');
  let inputs = [key(VK_LWIN, false), key(z, false), key(z, true), key(VK_LWIN, true)];
  // SAFETY: `inputs` ist ein gültiges Feld von `INPUT`-Strukturen; Länge und Strukturgröße stimmen.
  unsafe {
    SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32);
  }
}

/// Malt einen Punkt (unten rechts, mit transparentem Ring zum Symbol) in ein RGBA-Bild.
pub fn draw_badge(rgba: &mut [u8], width: u32, height: u32, color: [u8; 3]) {
  let size = width.min(height) as f32;
  let r = size * 0.24;
  let ring = (size * 0.07).max(1.0);
  let cx = width as f32 - r - ring * 0.5;
  let cy = height as f32 - r - ring * 0.5;
  for y in 0..height {
    for x in 0..width {
      let i = ((y * width + x) * 4) as usize;
      let Some(px) = rgba.get_mut(i..i + 4) else { return };
      let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
      // Ring ausstanzen (weiche Kante), dann den Punkt darüberlegen.
      let cut = (r + ring + 0.5 - d).clamp(0.0, 1.0);
      let base_a = f32::from(px[3]) / 255.0 * (1.0 - cut);
      let dot = (r + 0.5 - d).clamp(0.0, 1.0);
      let a = dot + base_a * (1.0 - dot);
      if a <= 0.0 {
        px.copy_from_slice(&[0, 0, 0, 0]);
        continue;
      }
      for c in 0..3 {
        let v = (f32::from(color[c]) * dot + f32::from(px[c]) * base_a * (1.0 - dot)) / a;
        px[c] = v.round().clamp(0.0, 255.0) as u8;
      }
      px[3] = (a * 255.0).round() as u8;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn badge_parse() {
    assert_eq!(Badge::parse("mention"), Badge::Mention);
    assert_eq!(Badge::parse("unread"), Badge::Unread);
    assert_eq!(Badge::parse("none"), Badge::None);
    assert_eq!(Badge::parse("quatsch"), Badge::None);
  }

  #[test]
  fn close_mode_roundtrip() {
    for mode in [CloseMode::Tray, CloseMode::Quit, CloseMode::Ask] {
      assert_eq!(CloseMode::from_u8(mode as u8), mode);
    }
    assert_eq!(CloseMode::parse("ask"), Some(CloseMode::Ask));
    assert_eq!(CloseMode::parse("quatsch"), None);
    // Unbekannter Wert → bisheriges Verhalten (verstecken).
    assert_eq!(CloseMode::from_u8(9), CloseMode::Tray);
  }

  #[test]
  fn minimized_arg() {
    assert!(started_minimized(["lykos.exe", "--minimized"]));
    assert!(!started_minimized(["lykos.exe"]));
  }

  #[test]
  fn badge_draws_dot_bottom_right_and_keeps_rest() {
    let (w, h) = (32u32, 32u32);
    let mut img = vec![10u8; (w * h * 4) as usize];
    for px in img.chunks_mut(4) {
      px[3] = 255;
    }
    draw_badge(&mut img, w, h, [255, 0, 0]);
    let at = |x: u32, y: u32| &img[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
    // Mitte des Punkts: volle Farbe.
    assert_eq!(at(24, 24), &[255, 0, 0, 255]);
    // Oben links: unverändert.
    assert_eq!(at(2, 2), &[10, 10, 10, 255]);
    // Zwischen Punkt und Symbol: ausgestanzter Ring (transparent).
    let ring = at(14, 24);
    assert!(ring[3] < 255, "Ring sollte (teil)transparent sein: {ring:?}");
  }
}
