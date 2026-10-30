//! Ablage für Dateien (Anhänge). Aufrufer kennen nur Schlüssel und Byte-Streams;
//! wo die Bytes liegen (lokales Dateisystem oder S3, siehe `s3_storage.rs`), entscheidet die Implementierung.
//!
//! Schlüssel sind ausschließlich serverseitig erzeugte IDs (UUIDs) – niemals Nutzereingaben.
//! `LocalStorage` prüft das zusätzlich, damit kein Pfad aus dem Speicherverzeichnis herausführt.

use std::{
    io,
    ops::Range,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt, future::BoxFuture};
use tokio::{
    fs,
    io::{AsyncSeekExt, AsyncWriteExt},
};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    config::{Config, StorageBackend},
    s3_storage::S3Storage,
};

/// Datenstrom, wie er beim Hoch- und Herunterladen durchgereicht wird.
pub type ByteStream<'a> = Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send + 'a>>;

/// Ein gespeichertes Objekt, wie es `list` liefert.
#[derive(Debug, Clone)]
pub struct StoredObject {
    pub key: String,
    /// Zeitpunkt des Schreibens; schützt laufende Uploads vor dem Aufräumen.
    pub modified: SystemTime,
}

/// Speicher-Backend. Bewusst objektorientiert geschnitten (put/get/delete/list per Schlüssel),
/// damit eine S3-Implementierung dieselbe Schnittstelle bedienen kann.
pub trait Storage: Send + Sync {
    /// Schreibt den Stream vollständig unter `key` und gibt die Anzahl Bytes zurück.
    /// Bricht der Stream mit einem Fehler ab, bleibt nichts liegen und der Fehler wird durchgereicht.
    fn put<'a>(&'a self, key: &'a str, data: ByteStream<'a>) -> BoxFuture<'a, io::Result<u64>>;

    /// Liest das Objekt (optional nur den Byte-Bereich `range`). `None`, wenn es nicht existiert.
    fn get<'a>(
        &'a self,
        key: &'a str,
        range: Option<Range<u64>>,
    ) -> BoxFuture<'a, io::Result<Option<ByteStream<'static>>>>;

    /// Löscht das Objekt; ein fehlendes Objekt ist kein Fehler.
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, io::Result<()>>;

    /// Alle gespeicherten Objekte (für den Abgleich mit der Datenbank).
    fn list(&self) -> BoxFuture<'_, io::Result<Vec<StoredObject>>>;

    /// Räumt abgebrochene, unfertige Schreibvorgänge auf, die älter als `older_than` sind.
    fn cleanup_incomplete(&self, _older_than: Duration) -> BoxFuture<'_, io::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// Prüft einen Speicher-Schlüssel: 3–64 Zeichen aus `[a-z0-9-]` (UUIDs). Gilt für alle Backends.
pub fn validate_key(key: &str) -> io::Result<()> {
    let valid = (3..=64).contains(&key.len())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "ungültiger Speicher-Schlüssel"))
    }
}

/// Speicher-Backend gemäß Konfiguration (`LYKOS_STORAGE`).
pub fn from_config(config: &Config) -> anyhow::Result<Arc<dyn Storage>> {
    Ok(match &config.storage {
        StorageBackend::Local => Arc::new(LocalStorage::new(config.storage_dir.clone())),
        StorageBackend::S3(s3) => Arc::new(S3Storage::new(s3)?),
    })
}

/// Speicher im lokalen Dateisystem: `<root>/<letzte 2 Zeichen>/<key>`, Uploads entstehen in
/// `<root>/.tmp/` und werden erst nach vollständigem Schreiben an ihren Platz verschoben.
pub struct LocalStorage {
    root: PathBuf,
}

const TMP_DIR: &str = ".tmp";

impl LocalStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Pfad eines Objekts. Nur gültige Schlüssel, damit kein Pfad aus `root` herausführt.
    fn path(&self, key: &str) -> io::Result<PathBuf> {
        validate_key(key)?;
        Ok(self.root.join(&key[key.len() - 2..]).join(key))
    }

    async fn write(&self, tmp: &Path, mut data: ByteStream<'_>) -> io::Result<u64> {
        let mut file = fs::File::create(tmp).await?;
        let mut written = 0u64;
        while let Some(chunk) = data.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
        }
        file.sync_all().await?;
        Ok(written)
    }
}

impl Storage for LocalStorage {
    fn put<'a>(&'a self, key: &'a str, data: ByteStream<'a>) -> BoxFuture<'a, io::Result<u64>> {
        Box::pin(async move {
            let target = self.path(key)?;
            let tmp_dir = self.root.join(TMP_DIR);
            fs::create_dir_all(&tmp_dir).await?;
            let tmp = tmp_dir.join(Uuid::now_v7().to_string());

            let result = async {
                let written = self.write(&tmp, data).await?;
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).await?;
                }
                fs::rename(&tmp, &target).await?;
                Ok(written)
            }
            .await;
            if result.is_err() {
                let _ = fs::remove_file(&tmp).await;
            }
            result
        })
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
        range: Option<Range<u64>>,
    ) -> BoxFuture<'a, io::Result<Option<ByteStream<'static>>>> {
        Box::pin(async move {
            let path = self.path(key)?;
            let mut file = match fs::File::open(&path).await {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            let stream: ByteStream<'static> = match range {
                Some(r) => {
                    file.seek(io::SeekFrom::Start(r.start)).await?;
                    let len = r.end.saturating_sub(r.start);
                    Box::pin(ReaderStream::new(tokio::io::AsyncReadExt::take(file, len)))
                }
                None => Box::pin(ReaderStream::new(file)),
            };
            Ok(Some(stream))
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move {
            match fs::remove_file(self.path(key)?).await {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        })
    }

    fn list(&self) -> BoxFuture<'_, io::Result<Vec<StoredObject>>> {
        Box::pin(async move {
            let mut out = Vec::new();
            let mut shards = match fs::read_dir(&self.root).await {
                Ok(d) => d,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
                Err(e) => return Err(e),
            };
            while let Some(shard) = shards.next_entry().await? {
                if shard.file_name() == TMP_DIR || !shard.file_type().await?.is_dir() {
                    continue;
                }
                let mut files = fs::read_dir(shard.path()).await?;
                while let Some(file) = files.next_entry().await? {
                    let Ok(key) = file.file_name().into_string() else { continue };
                    let meta = file.metadata().await?;
                    if meta.is_file() {
                        out.push(StoredObject { key, modified: meta.modified()? });
                    }
                }
            }
            Ok(out)
        })
    }

    fn cleanup_incomplete(&self, older_than: Duration) -> BoxFuture<'_, io::Result<()>> {
        Box::pin(async move {
            let mut entries = match fs::read_dir(self.root.join(TMP_DIR)).await {
                Ok(d) => d,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };
            let now = SystemTime::now();
            while let Some(entry) = entries.next_entry().await? {
                let modified = entry.metadata().await?.modified()?;
                if now.duration_since(modified).unwrap_or_default() > older_than {
                    let _ = fs::remove_file(entry.path()).await;
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(parts: Vec<&'static [u8]>) -> ByteStream<'static> {
        Box::pin(futures_util::stream::iter(parts.into_iter().map(|p| Ok(Bytes::from_static(p)))))
    }

    async fn read_all(s: ByteStream<'static>) -> Vec<u8> {
        s.map(|c| c.unwrap().to_vec()).concat().await
    }

    #[tokio::test]
    async fn roundtrip_range_list_delete() {
        let dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(dir.path());
        let key = Uuid::now_v7().to_string();

        assert_eq!(storage.put(&key, stream(vec![b"hallo ", b"welt"])).await.unwrap(), 10);
        assert_eq!(read_all(storage.get(&key, None).await.unwrap().unwrap()).await, b"hallo welt");
        assert_eq!(read_all(storage.get(&key, Some(6..10)).await.unwrap().unwrap()).await, b"welt");

        let listed = storage.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, key);

        storage.delete(&key).await.unwrap();
        storage.delete(&key).await.unwrap();
        assert!(storage.get(&key, None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn failed_upload_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(dir.path());
        let key = Uuid::now_v7().to_string();
        let failing: ByteStream<'static> = Box::pin(futures_util::stream::iter(vec![
            Ok(Bytes::from_static(b"abc")),
            Err(io::Error::other("abgebrochen")),
        ]));
        assert!(storage.put(&key, failing).await.is_err());
        assert!(storage.get(&key, None).await.unwrap().is_none());
        assert!(storage.list().await.unwrap().is_empty());
        let mut tmp = fs::read_dir(dir.path().join(TMP_DIR)).await.unwrap();
        assert!(tmp.next_entry().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_path_traversal_keys() {
        let dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(dir.path());
        for key in ["../../etc/passwd", "a/b", "..", "ABC", "x"] {
            assert!(storage.put(key, stream(vec![b"x"])).await.is_err(), "{key}");
            assert!(storage.get(key, None).await.is_err(), "{key}");
            assert!(storage.delete(key).await.is_err(), "{key}");
        }
    }
}
