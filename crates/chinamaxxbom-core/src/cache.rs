//! Shared persistent API cache backed by cached/redb.
use anyhow::{ensure, Context, Result};
use cached::{ConcurrentCachedExt, RedbCache};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
const MIN_REQUEST_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheInfo {
    pub hit: bool,
    pub fetched_at: u64,
    pub expires_at: u64,
}
#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    fetched_at: u64,
    data: String,
}
#[derive(Debug, Clone)]
pub struct Cache {
    directory: PathBuf,
    interval: Duration,
    ttl: Duration,
}
impl Cache {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            interval: MIN_REQUEST_INTERVAL,
            ttl: Duration::from_secs(TTL_SECONDS),
        }
    }
    fn user_directory() -> Result<Self> {
        if let Some(path) = std::env::var_os("CHINAMAXXBOM_CACHE_DIR").filter(|p| !p.is_empty()) {
            return Ok(Self::new(path.into()));
        }
        let root = if let Some(path) = std::env::var_os("XDG_CACHE_HOME").filter(|p| !p.is_empty())
        {
            PathBuf::from(path)
        } else if cfg!(target_os = "windows") {
            PathBuf::from(
                std::env::var_os("LOCALAPPDATA")
                    .context("LOCALAPPDATA not set; set CHINAMAXXBOM_CACHE_DIR")?,
            )
        } else {
            let home = PathBuf::from(
                std::env::var_os("HOME").context("HOME not set; set CHINAMAXXBOM_CACHE_DIR")?,
            );
            home.join(if cfg!(target_os = "macos") {
                "Library/Caches"
            } else {
                ".cache"
            })
        };
        Ok(Self::new(root.join("chinamaxxbom").join("api-v1")))
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn for_user() -> Result<Self> {
        let mut cache = Self::user_directory()?;
        let path = cache.directory.join("settings.json");
        match fs::read(&path) {
            Ok(bytes) => {
                let settings: Settings =
                    serde_json::from_slice(&bytes).context("Read cache settings")?;
                validate_days(settings.ttl_days)?;
                cache.ttl = Duration::from_secs(settings.ttl_days * 86400);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(cache)
    }
    pub fn ttl_days(&self) -> u64 {
        self.ttl.as_secs() / 86400
    }
    pub fn save_ttl_days(days: u64) -> Result<()> {
        validate_days(days)?;
        let cache = Self::user_directory()?;
        fs::create_dir_all(&cache.directory)?;
        crate::project::atomic_write(
            &cache.directory.join("settings.json"),
            &serde_json::to_vec_pretty(&Settings { ttl_days: days })?,
        )
    }
    pub fn get_or_fetch<T: Serialize + DeserializeOwned>(
        &self,
        key: &str,
        fetch: impl FnOnce() -> Result<T>,
    ) -> Result<(T, CacheInfo)> {
        fs::create_dir_all(&self.directory)
            .with_context(|| format!("Create API cache {}", self.directory.display()))?;
        // OS lock works across GUI/CLI processes. Recheck after locking so identical
        // simultaneous requests share one response. Dropping the file releases it.
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.directory.join("requests.lock"))?;
        lock.lock().context("Lock supplier cache")?;
        // Open under the process lock; redb permits only one database opener.
        let store: RedbCache<String, Entry> = RedbCache::builder("responses")
            .disk_dir(&self.directory)
            .ttl(self.ttl)
            .refresh_on_hit(false)
            .build()?;
        let key = key.to_owned();
        if let Some(entry) = store.get(&key)? {
            return Ok((
                serde_json::from_str(&entry.data)?,
                CacheInfo {
                    hit: true,
                    fetched_at: entry.fetched_at,
                    expires_at: entry.fetched_at.saturating_add(self.ttl.as_secs()),
                },
            ));
        }
        let last_path = self.directory.join("last-request-ms");
        let last = fs::read_to_string(&last_path)
            .ok()
            .and_then(|s| s.parse::<u64>().ok());
        let current = now().as_millis() as u64;
        if let Some(last) = last {
            let delay = request_delay(current, last, self.interval);
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        }
        crate::project::atomic_write(
            &last_path,
            (now().as_millis() as u64).to_string().as_bytes(),
        )?;
        // Errors are never cached as successes and there are no automatic retries.
        let value = fetch()?;
        let fetched_at = now().as_secs();
        let entry = Entry {
            fetched_at,
            data: serde_json::to_string(&value)?,
        };
        store.set(key, entry)?;
        Ok((
            value,
            CacheInfo {
                hit: false,
                fetched_at,
                expires_at: fetched_at.saturating_add(self.ttl.as_secs()),
            },
        ))
    }
}
fn now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}
fn request_delay(current: u64, last: u64, interval: Duration) -> Duration {
    interval.saturating_sub(Duration::from_millis(current.saturating_sub(last)))
}

#[derive(Serialize, Deserialize)]
struct Settings {
    ttl_days: u64,
}
fn validate_days(days: u64) -> Result<()> {
    ensure!(
        (7..=60).contains(&days),
        "Cache TTL must be between 7 and 60 days"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_cache_reuses_success_and_retries_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cache = Cache::new(tmp.path().into());
        cache.interval = Duration::ZERO;
        assert!(cache
            .get_or_fetch::<String>("q", || anyhow::bail!("429"))
            .is_err());
        let (_, first) = cache
            .get_or_fetch("q", || Ok("result".to_string()))
            .unwrap();
        assert!(!first.hit);
        let (value, second) = Cache::new(tmp.path().into())
            .get_or_fetch::<String>("q", || panic!("must reuse disk entry"))
            .unwrap();
        assert_eq!(value, "result");
        assert!(second.hit);
        assert_eq!(second.expires_at - second.fetched_at, TTL_SECONDS);
    }
    #[test]
    fn backend_expires_without_refreshing_on_hit() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cache = Cache::new(tmp.path().into());
        cache.interval = Duration::ZERO;
        cache.ttl = Duration::from_millis(100);
        cache.get_or_fetch("q", || Ok(1)).unwrap();
        assert!(
            cache
                .get_or_fetch::<i32>("q", || panic!("cache hit"))
                .unwrap()
                .1
                .hit
        );
        std::thread::sleep(Duration::from_millis(150));
        let (value, meta) = cache.get_or_fetch("q", || Ok(2)).unwrap();
        assert_eq!(value, 2);
        assert!(!meta.hit);
    }
    #[test]
    fn ttl_bounds() {
        for days in [7, 30, 60] {
            assert!(validate_days(days).is_ok());
        }
        for days in [0, 6, 61, u64::MAX] {
            assert!(validate_days(days).is_err());
        }
    }
}
