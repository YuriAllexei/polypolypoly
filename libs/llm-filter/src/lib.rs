pub mod cache;
pub mod ollama;

use cache::{CacheEntry, MarketCache};
use chrono::{Duration, Utc};
use ollama::OllamaClient;
use std::path::Path;
use thiserror::Error;
use tracing::{debug, info};

#[derive(Error, Debug)]
pub enum FilterError {
    #[error("Cache error: {0}")]
    CacheError(#[from] cache::CacheError),

    #[error("Ollama error: {0}")]
    OllamaError(#[from] ollama::OllamaError),
}

pub type Result<T> = std::result::Result<T, FilterError>;

/// Market information for filtering
#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub id: String,
    pub question: String,
    pub resolution_time: chrono::DateTime<Utc>,
}

/// LLM-based market filter with caching
pub struct LLMFilter {
    cache: MarketCache,
    ollama: OllamaClient,
    prompt: String,
}

impl LLMFilter {
    /// Create new LLM filter
    pub fn new(
        cache_path: impl AsRef<Path>,
        ollama_endpoint: impl Into<String>,
        ollama_model: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Result<Self> {
        let cache = MarketCache::load(cache_path)?;
        let ollama = OllamaClient::new(ollama_endpoint, ollama_model);
        let prompt = prompt.into();

        Ok(Self {
            cache,
            ollama,
            prompt,
        })
    }

    /// Filter markets using cache + LLM
    ///
    /// Returns list of compatible markets
    pub async fn filter_markets(&mut self, markets: Vec<MarketInfo>) -> Result<Vec<MarketInfo>> {
        if markets.is_empty() {
            return Ok(Vec::new());
        }

        info!("Filtering {} markets", markets.len());

        let mut compatible = Vec::new();
        let mut uncached = Vec::new();

        // 1. Check cache first
        for market in markets {
            if let Some(entry) = self.cache.get(&market.question) {
                if entry.compatible {
                    debug!("✓ Cache hit (compatible): {}", market.question);
                    compatible.push(market);
                } else {
                    debug!("✗ Cache hit (incompatible): {}", market.question);
                }
                // Skip if already checked (compatible or not)
            } else {
                debug!("? Cache miss: {}", market.question);
                uncached.push(market);
            }
        }

        info!(
            "Cache: {} hits, {} misses",
            compatible.len(),
            uncached.len()
        );

        // 2. Send uncached markets to LLM
        if !uncached.is_empty() {
            info!("Querying LLM for {} uncached markets", uncached.len());

            // Prepare market data for LLM
            let market_data: Vec<(String, String)> = uncached
                .iter()
                .map(|m| (m.id.clone(), m.question.clone()))
                .collect();

            // Get LLM results
            let llm_results = self.ollama.filter_markets(&self.prompt, &market_data).await?;

            // 3. Update cache with results
            for market in uncached {
                let is_compatible = llm_results.contains(&market.id);

                let entry = CacheEntry {
                    market_id: market.id.clone(),
                    question: market.question.clone(),
                    compatible: is_compatible,
                    checked_at: Utc::now(),
                    resolution_time: market.resolution_time,
                };

                debug!(
                    "{} LLM result: {}",
                    if is_compatible { "✓" } else { "✗" },
                    market.question
                );

                self.cache.insert(market.question.clone(), entry);

                if is_compatible {
                    compatible.push(market);
                }
            }

            // 4. Save updated cache
            self.cache.save()?;

            info!(
                "LLM identified {}/{} as compatible",
                llm_results.len(),
                market_data.len()
            );
        }

        info!("Total compatible markets: {}", compatible.len());

        Ok(compatible)
    }

    /// Clean old cache entries (older than max_age)
    pub fn cleanup_cache(&mut self, max_age: Duration) -> Result<()> {
        self.cache.cleanup_old_entries(max_age);
        self.cache.save()?;
        Ok(())
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> cache::CacheStats {
        self.cache.stats()
    }

    /// Check if LLM is available
    pub async fn health_check(&self) -> Result<bool> {
        Ok(self.ollama.health_check().await?)
    }

    /// Force save cache
    pub fn save_cache(&mut self) -> Result<()> {
        Ok(self.cache.save()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_filter_creation() {
        let temp = NamedTempFile::new().unwrap();

        let filter = LLMFilter::new(
            temp.path(),
            "http://localhost:11434",
            "llama3.2",
            "Test prompt",
        );

        assert!(filter.is_ok());
    }
}
