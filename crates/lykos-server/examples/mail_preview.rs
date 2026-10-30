//! Rendert alle Mail-Vorlagen mit Beispieldaten als Dateien zum Ansehen im Browser – ohne Mailversand.
//!
//! ```text
//! cargo run -p lykos-server --example mail_preview [ZIELORDNER]
//! ```
//!
//! Standard-Ziel: `$CARGO_TARGET_DIR/mail-preview` bzw. `target/mail-preview`. Je Mail entstehen
//! `<name>.html` (Logo statt `cid:` als `logo.png` daneben) und `<name>.txt` (Betreff + Textteil),
//! dazu `index.html` mit allen Mails. Dunkel-Ansicht: im Browser `prefers-color-scheme: dark` emulieren.

use std::{fs, path::PathBuf};

use lykos_server::mail;

fn main() -> std::io::Result<()> {
    let dir = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into()))
            .join("mail-preview")
    });
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("logo.png"), mail::LOGO_PNG)?;

    let mut index = String::from(
        "<!DOCTYPE html><html lang=\"de\"><head><meta charset=\"utf-8\"><title>Lykos – Mail-Vorschau</title>\
         <style>body{font-family:system-ui,sans-serif;margin:24px}iframe{width:640px;height:900px;border:1px solid #ccc;\
         margin:0 16px 24px 0;vertical-align:top}</style></head><body><h1>Lykos – Mail-Vorschau</h1>\n",
    );
    for (name, m) in mail::preview_samples() {
        let html = m.html.replace(&format!("cid:{}", mail::LOGO_CID), "logo.png");
        fs::write(dir.join(format!("{name}.html")), html)?;
        fs::write(dir.join(format!("{name}.txt")), format!("Betreff: {}\n\n{}", m.subject, m.text))?;
        index.push_str(&format!(
            "<h2>{name} – {subject}</h2><iframe src=\"{name}.html\"></iframe>\
             <iframe src=\"{name}.txt\"></iframe>\n",
            subject = m.subject
        ));
        println!("{}", dir.join(format!("{name}.html")).display());
    }
    index.push_str("</body></html>\n");
    fs::write(dir.join("index.html"), index)?;
    println!("{}", dir.join("index.html").display());
    Ok(())
}
