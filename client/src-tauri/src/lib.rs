//! Desktop-Hülle: Session-Token im OS-Schlüsselbund (Windows-Anmeldeinformationsverwaltung,
//! macOS-Schlüsselbund, Secret Service unter Linux), Desktop-Benachrichtigungen, globales
//! Push-to-Talk, Auto-Update, Infobereich/Einzelinstanz/Autostart (siehe `desktop.rs`) und (Windows)
//! Mikrofon/Kamera ohne WebView2-Rückfrage.

#[cfg(desktop)]
mod desktop;
mod permissions;

/// Dienstname der Einträge im Schlüsselbund. Der Kontoname ist die Server-URL der Instanz,
/// damit jede Instanz ihr eigenes Token hat.
const KEYRING_SERVICE: &str = "dev.zeasy.lykos";

fn entry(server: &str) -> Result<keyring::Entry, String> {
  keyring::Entry::new(KEYRING_SERVICE, server).map_err(|e| e.to_string())
}

/// Token für eine Instanz lesen; `None`, wenn keins gespeichert ist.
#[tauri::command]
fn token_get(server: String) -> Result<Option<String>, String> {
  match entry(&server)?.get_password() {
    Ok(token) => Ok(Some(token)),
    Err(keyring::Error::NoEntry) => Ok(None),
    Err(e) => Err(e.to_string()),
  }
}

#[tauri::command]
fn token_set(server: String, token: String) -> Result<(), String> {
  entry(&server)?.set_password(&token).map_err(|e| e.to_string())
}

#[tauri::command]
fn token_delete(server: String) -> Result<(), String> {
  match entry(&server)?.delete_credential() {
    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
    Err(e) => Err(e.to_string()),
  }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let builder = tauri::Builder::default();
  // Desktop: Einzelinstanz zuerst registrieren – ein zweiter Start holt nur das vorhandene Fenster nach vorn.
  #[cfg(desktop)]
  let builder = builder
    .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
      desktop::on_second_instance(app, args, cwd)
    }))
    .plugin(tauri_plugin_autostart::Builder::new().args([desktop::MINIMIZED_ARG]).build())
    .on_window_event(desktop::on_window_event)
    .invoke_handler(tauri::generate_handler![
      token_get,
      token_set,
      token_delete,
      desktop::desktop_set_close_mode,
      desktop::desktop_close_ack,
      desktop::desktop_close_resolve,
      desktop::desktop_tray_update,
      desktop::desktop_autostart_get,
      desktop::desktop_autostart_set,
      desktop::desktop_snap_overlay,
    ]);
  #[cfg(not(desktop))]
  let builder = builder.invoke_handler(tauri::generate_handler![token_get, token_set, token_delete]);

  builder
    .plugin(tauri_plugin_notification::init())
    .plugin(tauri_plugin_opener::init())
    .setup(|app| {
      // Desktop: globales Push-to-Talk, Auto-Update (signierte Manifeste) und Neustart danach;
      // Tray-Symbol und erstes Anzeigen des Fensters (nicht beim Autostart).
      #[cfg(desktop)]
      {
        app.handle().plugin(tauri_plugin_global_shortcut::Builder::new().build())?;
        app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
        app.handle().plugin(tauri_plugin_process::init())?;
        desktop::setup(app)?;
      }
      // Windows: Mikrofon/Kamera für die eigene App-Herkunft ohne WebView2-Popup (siehe permissions.rs).
      #[cfg(windows)]
      {
        use tauri::Manager;
        let dev_url = app.config().build.dev_url.as_ref().map(|u| u.to_string());
        if let Some(window) = app.get_webview_window("main") {
          permissions::auto_grant_media(&window, permissions::app_origins(dev_url.as_deref()));
        }
      }
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while building tauri application");
}
