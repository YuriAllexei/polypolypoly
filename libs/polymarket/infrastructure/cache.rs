// Re-export domain types for application layer
pub use crate::domain::filter::{CacheEntry, CacheError, CacheStats};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

pub type Result<T> = std::result::Result<T, CacheError>;

// CacheEntry is now defined in domain::filter

// Make CacheEntry serializable for infrastructure

/// Market cache manager
pub struct MarketCache {
    /// Cache entries keyed by market question (used as unique identifier)
    cache: HashMap<String, CacheEntry>,

    /// Path to cache file
    file_path: PathBuf,

    /// Whether cache has been modified
    dirty: bool,
}

impl MarketCache {
    /// Load cache from JSON file
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let file_path = path.as_ref().to_path_buf();

        let cache = if file_path.exists() {
            debug!("Loading cache from {:?}", file_path);
            let content = fs::read_to_string(&file_path)
                .map_err(CacheError::from_io_error)?;
            serde_json::from_str(&content)
                .map_err(CacheError::from_json_error)?
        } else {
            debug!("Cache file not found, starting with empty cache");
            HashMap::new()
        };

        debug!("Loaded {} entries from cache", cache.len());

        Ok(Self {
            cache,
            file_path,
            dirty: false,
        })
    }

    /// Check if market question exists in cache
    pub fn is_cached(&self, question: &str) -> bool {
        self.cache.contains_key(question)
    }

    /// Get cached entry by question
    pub fn get(&self, question: &str) -> Option<&CacheEntry> {
        self.cache.get(question)
    }

    /// Insert or update entry
    pub fn insert(&mut self, question: String, entry: CacheEntry) {
        self.cache.insert(question, entry);
        self.dirty = true;
    }

    /// Remove entry by question
    pub fn remove(&mut self, question: &str) -> Option<CacheEntry> {
        self.dirty = true;
        self.cache.remove(question)
    }

    /// Save cache to JSON file
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            debug!("Cache not modified, skipping save");
            return Ok(());
        }

        debug!(
            "Saving {} entries to cache file {:?}",
            self.cache.len(),
            self.file_path
        );

        let json = serde_json::to_string_pretty(&self.cache)
            .map_err(CacheError::from_json_error)?;
        fs::write(&self.file_path, json)
            .map_err(CacheError::from_io_error)?;

        self.dirty = false;
        Ok(())
    }

    /// Clean old entries older than max_age
    pub fn cleanup_old_entries(&mut self, max_age: Duration) {
        let now = Utc::now();
        let cutoff = now - max_age;

        let before_count = self.cache.len();

        self.cache.retain(|_, entry| {
            // Keep if checked recently
            if entry.checked_at > cutoff {
                return true;
            }

            // Also keep if market hasn't resolved yet (might still be useful)
            if entry.resolution_time > now {
                return true;
            }

            false
        });

        let removed = before_count - self.cache.len();
        if removed > 0 {
            debug!("Cleaned up {} old cache entries", removed);
            self.dirty = true;
        }
    }

    /// Get all compatible markets
    pub fn get_compatible_markets(&self) -> Vec<&CacheEntry> {
        self.cache
            .values()
            .filter(|entry| entry.compatible)
            .collect()
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let total = self.cache.len();
        let compatible = self.cache.values().filter(|e| e.compatible).count();
        let incompatible = total - compatible;

        CacheStats {
            total,
            compatible,
            incompatible,
        }
    }

    /// Get number of entries in cache
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Drop for MarketCache {
    fn drop(&mut self) {
        // Auto-save on drop
        if self.dirty {
            if let Err(e) = self.save() {
                warn!("Failed to save cache on drop: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_cache_load_empty() {
        let temp = NamedTempFile::new().unwrap();
        let cache = MarketCache::load(temp.path()).unwrap();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_insert_and_get() {
        let temp = NamedTempFile::new().unwrap();
        let mut cache = MarketCache::load(temp.path()).unwrap();

        let entry = CacheEntry {
            market_id: "0x123".to_string(),
            question: "Will BTC go up?".to_string(),
            compatible: true,
            checked_at: Utc::now(),
            resolution_time: Utc::now() + Duration::hours(1),
        };

        cache.insert("Will BTC go up?".to_string(), entry.clone());

        assert!(cache.is_cached("Will BTC go up?"));
        assert!(cache.get("Will BTC go up?").is_some());
    }

    #[test]
    fn test_cache_save_and_load() {
        let temp = NamedTempFile::new().unwrap();

        // Create and save cache
        {
            let mut cache = MarketCache::load(temp.path()).unwrap();
            let entry = CacheEntry {
                market_id: "0x123".to_string(),
                question: "Test question".to_string(),
                compatible: true,
                checked_at: Utc::now(),
                resolution_time: Utc::now() + Duration::hours(1),
            };
            cache.insert("Test question".to_string(), entry);
            cache.save().unwrap();
        }

        // Load and verify
        {
            let cache = MarketCache::load(temp.path()).unwrap();
            assert_eq!(cache.len(), 1);
            assert!(cache.is_cached("Test question"));
        }
    }
}
