use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::{StreamExt, TryStreamExt};
use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};
use crate::paths::ensure_parent;

const CONCURRENCY: usize = 16;
const RETRIES: usize = 3;

/// Progress callbacks, so the CLI and a future GUI can render the same events.
#[derive(Debug, Clone)]
pub enum Progress {
    /// A new stage started; `total` is the number of items in it.
    Stage { name: String, total: u64 },
    /// One item of the current stage finished.
    Advance,
}

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

pub fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

#[derive(Debug, Clone)]
pub struct Download {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("netherNode/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .expect("failed to build HTTP client")
}

pub async fn get_json<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
) -> Result<T> {
    Ok(http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

pub fn sha1_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Cheap "already have it" check: the file exists and its size matches.
/// Full hashing happens right after each download instead of on every launch.
fn is_present(d: &Download) -> bool {
    match std::fs::metadata(&d.dest) {
        Ok(meta) => d.size.is_none_or(|s| s == meta.len()),
        Err(_) => false,
    }
}

async fn fetch_once(http: &reqwest::Client, d: &Download) -> Result<()> {
    ensure_parent(&d.dest)?;
    let mut tmp = d.dest.clone().into_os_string();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let resp = http.get(&d.url).send().await?.error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut hasher = Sha1::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);

    if let Some(expected) = &d.sha1 {
        let actual = hex::encode(hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(Error::Checksum {
                url: d.url.clone(),
                expected: expected.clone(),
                actual,
            });
        }
    }
    tokio::fs::rename(&tmp, &d.dest).await?;
    Ok(())
}

pub async fn fetch(http: &reqwest::Client, d: &Download) -> Result<()> {
    let mut last_err = None;
    for attempt in 0..RETRIES {
        match fetch_once(http, d).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
            }
        }
    }
    Err(last_err.expect("at least one attempt"))
}

/// Download everything that is missing, in parallel. Reports one `Advance` per item.
pub async fn fetch_all(
    http: &reqwest::Client,
    stage: &str,
    items: Vec<Download>,
    progress: &ProgressFn,
) -> Result<()> {
    progress(Progress::Stage {
        name: stage.to_string(),
        total: items.len() as u64,
    });
    futures::stream::iter(items)
        .map(|d| async move {
            if !is_present(&d) {
                fetch(http, &d).await?;
            }
            progress(Progress::Advance);
            Ok::<_, Error>(())
        })
        .buffer_unordered(CONCURRENCY)
        .try_collect::<()>()
        .await
}
