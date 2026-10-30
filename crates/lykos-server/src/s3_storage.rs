//! S3-kompatibler Speicher (Garage, MinIO, AWS S3) für den `Storage`-Trait.
//!
//! Umsetzung mit `object_store` (Apache Arrow): schlanker S3-Client auf dem ohnehin vorhandenen
//! `reqwest`, Kryptografie über `ring` – kein AWS-SDK, keine C-Toolchain (aws-lc-rs) nötig.
//!
//! - **Hochladen:** Kleine Dateien (bis `MULTIPART_THRESHOLD`) gehen in einem `PUT` hoch. Größere
//!   werden per Multipart-Upload gestreamt, ohne die ganze Datei im Speicher zu halten; bricht der
//!   Datenstrom ab, wird der Multipart-Upload abgebrochen (es bleibt nichts sichtbar liegen).
//! - **Herunterladen:** Byte-Bereiche werden als `Range`-Request an S3 weitergereicht.
//! - **Aufräumen:** Abgestürzte Multipart-Uploads kann `object_store` nicht auflisten – dafür im Bucket
//!   eine Lifecycle-Regel `AbortIncompleteMultipartUpload` setzen (siehe README).
//!
//! Der Bucket (bzw. das Präfix) gehört Lykos allein: Der Aufräum-Job löscht dort Objekte mit
//! Lykos-Schlüsselformat, zu denen es keinen Anhang mehr gibt.

use std::{io, ops::Range, sync::Arc, time::Duration};

use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt, future::BoxFuture};
use object_store::{
    GetOptions, GetRange, ObjectStore, ObjectStoreExt, PutPayload, RetryConfig, WriteMultipart,
    aws::AmazonS3Builder, path::Path,
};

use crate::{
    config::S3Config,
    storage::{ByteStream, Storage, StoredObject, validate_key},
};

/// Bis zu dieser Größe ein einfacher `PUT`, darüber Multipart (S3-Mindestgröße je Teil: 5 MiB).
const MULTIPART_THRESHOLD: usize = 8 * 1024 * 1024;
/// Größe der Multipart-Teile.
const PART_SIZE: usize = 8 * 1024 * 1024;
/// Höchstens so viele Teile gleichzeitig unterwegs (begrenzt den Speicherbedarf pro Upload).
const MAX_PARTS_IN_FLIGHT: usize = 2;

pub struct S3Storage {
    store: Arc<dyn ObjectStore>,
    prefix: Option<Path>,
}

impl S3Storage {
    pub fn new(config: &S3Config) -> anyhow::Result<Self> {
        crate::install_crypto_provider();
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&config.bucket)
            .with_region(&config.region)
            .with_access_key_id(&config.access_key_id)
            .with_secret_access_key(&config.secret_access_key)
            .with_virtual_hosted_style_request(!config.path_style)
            // Lieber schnell scheitern als Uploads minutenlang hängen lassen.
            .with_retry(RetryConfig {
                max_retries: 3,
                retry_timeout: Duration::from_secs(30),
                ..Default::default()
            });
        if let Some(endpoint) = &config.endpoint {
            builder = builder
                .with_endpoint(endpoint_url(endpoint, &config.bucket, config.path_style))
                .with_allow_http(endpoint.starts_with("http://"));
        }
        let store = builder.build().map_err(|e| anyhow::anyhow!("S3-Speicher nicht konfigurierbar: {e}"))?;
        Ok(Self { store: Arc::new(store), prefix: config.prefix.as_deref().map(Path::from) })
    }

    fn location(&self, key: &str) -> io::Result<Path> {
        validate_key(key)?;
        Ok(match &self.prefix {
            Some(prefix) => prefix.clone().join(key),
            None => Path::from(key),
        })
    }

    async fn put_stream(&self, location: &Path, mut data: ByteStream<'_>) -> io::Result<u64> {
        // Erst puffern: Passt alles unter die Schwelle, genügt ein einzelner PUT.
        let mut head: Vec<Bytes> = Vec::new();
        let mut buffered = 0usize;
        while buffered <= MULTIPART_THRESHOLD {
            match data.next().await {
                Some(chunk) => {
                    let chunk = chunk?;
                    buffered += chunk.len();
                    head.push(chunk);
                }
                None => {
                    let payload: PutPayload = head.into_iter().collect();
                    self.store.put(location, payload).await.map_err(to_io)?;
                    return Ok(buffered as u64);
                }
            }
        }

        let upload = self.store.put_multipart(location).await.map_err(to_io)?;
        let mut writer = WriteMultipart::new_with_chunk_size(upload, PART_SIZE);
        let result = async {
            let mut written = 0u64;
            for chunk in head {
                written += chunk.len() as u64;
                writer.put(chunk);
            }
            while let Some(chunk) = data.next().await {
                let chunk = chunk?;
                writer.wait_for_capacity(MAX_PARTS_IN_FLIGHT).await.map_err(to_io)?;
                written += chunk.len() as u64;
                writer.put(chunk);
            }
            Ok::<_, io::Error>(written)
        }
        .await;
        match result {
            Ok(written) => {
                writer.finish().await.map_err(to_io)?;
                Ok(written)
            }
            Err(e) => {
                if let Err(abort) = writer.abort().await {
                    tracing::warn!(error = %abort, "S3-Multipart-Upload ließ sich nicht abbrechen");
                }
                Err(e)
            }
        }
    }
}

/// Endpunkt für `object_store`: Im Pfad-Stil unverändert, bei virtuellen Hosts mit dem Bucket als
/// Subdomain (`https://s3.example.org` → `https://lykos.s3.example.org`).
fn endpoint_url(endpoint: &str, bucket: &str, path_style: bool) -> String {
    match endpoint.split_once("://") {
        Some((scheme, host)) if !path_style => format!("{scheme}://{bucket}.{host}"),
        _ => endpoint.to_owned(),
    }
}

fn to_io(e: object_store::Error) -> io::Error {
    match e {
        object_store::Error::NotFound { .. } => io::Error::new(io::ErrorKind::NotFound, e),
        e => io::Error::other(e),
    }
}

impl Storage for S3Storage {
    fn put<'a>(&'a self, key: &'a str, data: ByteStream<'a>) -> BoxFuture<'a, io::Result<u64>> {
        Box::pin(async move {
            let location = self.location(key)?;
            self.put_stream(&location, data).await
        })
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
        range: Option<Range<u64>>,
    ) -> BoxFuture<'a, io::Result<Option<ByteStream<'static>>>> {
        Box::pin(async move {
            let location = self.location(key)?;
            let options = GetOptions { range: range.map(GetRange::Bounded), ..Default::default() };
            match self.store.get_opts(&location, options).await {
                Ok(result) => Ok(Some(Box::pin(result.into_stream().map_err(to_io)) as ByteStream<'static>)),
                Err(object_store::Error::NotFound { .. }) => Ok(None),
                Err(e) => Err(to_io(e)),
            }
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move {
            let location = self.location(key)?;
            match self.store.delete(&location).await {
                Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
                Err(e) => Err(to_io(e)),
            }
        })
    }

    fn list(&self) -> BoxFuture<'_, io::Result<Vec<StoredObject>>> {
        Box::pin(async move {
            let depth = self.prefix.as_ref().map_or(0, |p| p.parts().count());
            let mut out = Vec::new();
            let mut objects = self.store.list(self.prefix.as_ref());
            while let Some(meta) = objects.next().await {
                let meta = meta.map_err(to_io)?;
                // Nur direkte Kinder des Präfixes im Lykos-Schlüsselformat – fremde Objekte bleiben unberührt.
                if meta.location.parts().count() != depth + 1 {
                    continue;
                }
                let Some(key) = meta.location.filename().map(str::to_owned) else { continue };
                if validate_key(&key).is_ok() {
                    out.push(StoredObject { key, modified: meta.last_modified.into() });
                }
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(prefix: Option<&str>) -> S3Config {
        S3Config {
            endpoint: Some("http://127.0.0.1:9".into()),
            bucket: "lykos".into(),
            region: "garage".into(),
            access_key_id: "GK".into(),
            secret_access_key: "geheim".into(),
            path_style: true,
            prefix: prefix.map(Into::into),
        }
    }

    #[test]
    fn locations_respect_prefix_and_reject_bad_keys() {
        let plain = S3Storage::new(&config(None)).unwrap();
        assert_eq!(
            plain.location("0190f000-0000-7000-8000-000000000001").unwrap().as_ref(),
            "0190f000-0000-7000-8000-000000000001"
        );
        let prefixed = S3Storage::new(&config(Some("lykos/anhaenge"))).unwrap();
        assert_eq!(prefixed.location("abc").unwrap().as_ref(), "lykos/anhaenge/abc");
        for key in ["../x", "a/b", "ABC", "x"] {
            assert!(prefixed.location(key).is_err(), "{key}");
        }
    }

    #[test]
    fn endpoint_for_addressing_style() {
        assert_eq!(endpoint_url("http://garage:3900", "lykos", true), "http://garage:3900");
        assert_eq!(endpoint_url("https://s3.example.org", "lykos", false), "https://lykos.s3.example.org");
    }
}
