//! Medien-Berechtigungen ohne WebView2-Rückfrage (nur Windows).
//!
//! WebView2 fragt sonst bei jedem `getUserMedia` mit einem eigenen Popup „… möchte Mikrofon/Kamera
//! verwenden“. Für die **eigene App-Herkunft** (`http://tauri.localhost` bzw. `https://…`, im Dev-Modus
//! die `devUrl`) erlauben wir Mikrofon, Kamera und Benachrichtigungen automatisch. Zwischenablage-Lesen
//! (`navigator.clipboard.read*`) braucht der Client nicht – Einfügen per Strg+V geht ohne Berechtigung –,
//! es bleibt daher beim WebView2-Standard (Rückfrage).
//!
//! Warum das sicher ist: Das Fenster lädt nur die eingebettete Oberfläche der App (kein fremder Inhalt,
//! keine Navigation auf andere Seiten – Links öffnen über `tauri-plugin-opener` im Systembrowser). Andere
//! Herkünfte (z. B. ein eingebetteter Rahmen) bekommen Mikrofon, Kamera und Benachrichtigungen nie;
//! alle übrigen Arten bleiben beim WebView2-Standard. Die Bildschirmauswahl (`getDisplayMedia`) ist keine
//! Berechtigung dieses Ereignisses und bleibt ein Auswahldialog. Der Windows-Datenschutzschalter für
//! Kamera/Mikrofon gilt weiterhin.

// Auf anderen Plattformen nur von Tests benutzt.
#![cfg_attr(not(windows), allow(dead_code))]

/// Herkünfte (`scheme://host[:port]`), denen die App-Berechtigungen ohne Rückfrage zustehen.
pub(crate) fn app_origins(dev_url: Option<&str>) -> Vec<String> {
  let mut origins = vec![
    "http://tauri.localhost".to_owned(),
    "https://tauri.localhost".to_owned(),
    "tauri://localhost".to_owned(),
  ];
  if cfg!(debug_assertions)
    && let Some(origin) = dev_url.and_then(origin_of)
  {
    origins.push(origin);
  }
  origins
}

/// `scheme://host[:port]` einer URL (klein geschrieben); `None` ohne `://`.
pub(crate) fn origin_of(uri: &str) -> Option<String> {
  let (scheme, rest) = uri.split_once("://")?;
  let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
  // Benutzerdaten (`user@host`) gehören nicht zur Herkunft.
  let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
  if scheme.is_empty() || authority.is_empty() {
    return None;
  }
  Some(format!("{}://{}", scheme.to_ascii_lowercase(), authority.to_ascii_lowercase()))
}

/// Gehört `uri` zur App selbst?
pub(crate) fn is_app_origin(uri: &str, origins: &[String]) -> bool {
  origin_of(uri).is_some_and(|o| origins.contains(&o))
}

/// Registriert den `PermissionRequested`-Handler am WebView2 des Fensters.
#[cfg(windows)]
pub(crate) fn auto_grant_media<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, origins: Vec<String>) {
  use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
    COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
    COREWEBVIEW2_PERMISSION_STATE_ALLOW, COREWEBVIEW2_PERMISSION_STATE_DENY,
    ICoreWebView2PermissionRequestedEventArgs3,
  };
  use webview2_com::{PermissionRequestedEventHandler, take_pwstr};
  use windows_core::{Interface, PWSTR};

  let result = window.with_webview(move |webview| {
    let register = || -> windows_core::Result<()> {
      let core = unsafe { webview.controller().CoreWebView2()? };
      let handler = PermissionRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
        let mut uri = PWSTR::null();
        unsafe {
          args.PermissionKind(&mut kind)?;
          args.Uri(&mut uri)?;
        }
        let uri = take_pwstr(uri);
        let granted = kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
          || kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA
          || kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS;
        if !granted {
          // Alle anderen Arten: WebView2-Standard.
          return Ok(());
        }
        let own = is_app_origin(&uri, &origins);
        let state = if own { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DENY };
        unsafe {
          args.SetState(state)?;
          // Erlaubnis im WebView2-Profil merken; Ablehnungen nicht.
          if let Ok(args3) = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
            let _ = args3.SetSavesInProfile(own);
          }
        }
        Ok(())
      }));
      let mut token = 0i64;
      unsafe { core.add_PermissionRequested(&handler, &mut token) }
    };
    if let Err(e) = register() {
      log::warn!("WebView2-Berechtigungshandler nicht registriert: {e}");
    }
  });
  if let Err(e) = result {
    log::warn!("WebView2 nicht erreichbar: {e}");
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn herkunft() {
    assert_eq!(origin_of("http://tauri.localhost/index.html").as_deref(), Some("http://tauri.localhost"));
    assert_eq!(origin_of("HTTP://Tauri.LocalHost").as_deref(), Some("http://tauri.localhost"));
    assert_eq!(origin_of("http://localhost:5173/?x=1").as_deref(), Some("http://localhost:5173"));
    assert_eq!(origin_of("https://evil@tauri.localhost/").as_deref(), Some("https://tauri.localhost"));
    assert_eq!(origin_of("about:blank"), None);
    assert_eq!(origin_of("http:///pfad"), None);
  }

  #[test]
  fn nur_eigene_herkunft() {
    let origins = app_origins(Some("http://localhost:5173"));
    assert!(is_app_origin("http://tauri.localhost/", &origins));
    assert!(is_app_origin("https://tauri.localhost/kanal", &origins));
    assert!(!is_app_origin("http://tauri.localhost.evil.com/", &origins));
    assert!(!is_app_origin("https://lykos.zeasy.dev/", &origins));
    assert!(!is_app_origin("http://tauri.localhost:8080/", &origins));
    // devUrl nur in Debug-Builds
    assert_eq!(is_app_origin("http://localhost:5173/", &origins), cfg!(debug_assertions));
  }
}
