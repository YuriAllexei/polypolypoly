use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Error, Debug)]
pub enum OllamaError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Failed to parse response: {0}")]
    ParseError(String),
}

pub type Result<T> = std::result::Result<T, OllamaError>;

/// Ollama API client
pub struct OllamaClient {
    endpoint: String,
    model: String,
    client: Client,
}

/// Request to Ollama generate endpoint
#[derive(Debug, Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    stream: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<GenerateOptions>,
}

/// Generation options
#[derive(Debug, Serialize)]
struct GenerateOptions {
    temperature: f32,
    top_p: f32,

    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
}

/// Response from Ollama generate endpoint
#[derive(Debug, Deserialize)]
struct GenerateResponse {
    response: String,

    #[serde(default)]
    done: bool,
}

impl OllamaClient {
    /// Create new Ollama client
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            client: Client::new(),
        }
    }

    /// Generate completion from prompt
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        let url = format!("{}/api/generate", self.endpoint);

        debug!("Sending prompt to Ollama (model: {})", self.model);

        let request = GenerateRequest {
            model: self.model.clone(),
            prompt: prompt.to_string(),
            stream: false,
            options: Some(GenerateOptions {
                temperature: 0.1,  // Low temperature for more consistent filtering
                top_p: 0.9,
                num_predict: Some(1000),  // Limit response length
            }),
        };

        let response = self.client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(OllamaError::ApiError(format!(
                "Ollama request failed: {}",
                error_text
            )));
        }

        let generate_response: GenerateResponse = response
            .json()
            .await
            .map_err(|e| OllamaError::ParseError(e.to_string()))?;

        debug!("Received response from Ollama ({} chars)", generate_response.response.len());

        Ok(generate_response.response)
    }

    /// Filter markets using LLM
    ///
    /// Returns market IDs that the LLM identified as compatible
    pub async fn filter_markets(
        &self,
        system_prompt: &str,
        markets: &[(String, String)],  // (market_id, question)
    ) -> Result<Vec<String>> {
        if markets.is_empty() {
            return Ok(Vec::new());
        }

        // Build the prompt
        let mut prompt = format!("{}\n\n", system_prompt);
        prompt.push_str("Markets to analyze:\n");
        for (i, (id, question)) in markets.iter().enumerate() {
            prompt.push_str(&format!("{}. [ID: {}] {}\n", i + 1, id, question));
        }
        prompt.push_str("\nRespond with ONLY the market IDs (one per line) that match the criteria. Do not include explanations or numbering.");

        debug!("Filtering {} markets with LLM", markets.len());

        // Get LLM response
        let response = self.generate(&prompt).await?;

        // Parse response - extract market IDs
        let compatible_ids = self.parse_market_ids(&response, markets);

        debug!(
            "LLM identified {}/{} markets as compatible",
            compatible_ids.len(),
            markets.len()
        );

        Ok(compatible_ids)
    }

    /// Parse market IDs from LLM response
    fn parse_market_ids(&self, response: &str, markets: &[(String, String)]) -> Vec<String> {
        let mut compatible = Vec::new();

        // Create a set of valid market IDs for quick lookup
        let valid_ids: std::collections::HashSet<_> =
            markets.iter().map(|(id, _)| id.as_str()).collect();

        // Parse each line
        for line in response.lines() {
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Try to extract ID (handle various formats)
            let potential_id = line
                .trim_start_matches('[')
                .trim_start_matches("ID:")
                .trim_start_matches(']')
                .trim();

            // Check if this is a valid market ID
            if valid_ids.contains(potential_id) {
                compatible.push(potential_id.to_string());
            } else {
                // Try to find ID within the line
                for (id, _) in markets {
                    if line.contains(id) {
                        compatible.push(id.clone());
                        break;
                    }
                }
            }
        }

        // Remove duplicates
        compatible.sort();
        compatible.dedup();

        compatible
    }

    /// Check if Ollama is running and model is available
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/api/tags", self.endpoint);

        debug!("Checking Ollama health");

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Ok(false);
        }

        // Check if our model is in the list
        let tags_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| OllamaError::ParseError(e.to_string()))?;

        if let Some(models) = tags_response.get("models").and_then(|m| m.as_array()) {
            for model in models {
                if let Some(name) = model.get("name").and_then(|n| n.as_str()) {
                    if name.contains(&self.model) {
                        debug!("Model {} found in Ollama", self.model);
                        return Ok(true);
                    }
                }
            }
        }

        warn!("Model {} not found in Ollama. Please pull it first: docker exec -it polymarket-ollama ollama pull {}", self.model, self.model);
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_market_ids() {
        let client = OllamaClient::new("http://localhost:11434", "llama3.2");

        let markets = vec![
            ("0x123".to_string(), "Market 1".to_string()),
            ("0x456".to_string(), "Market 2".to_string()),
            ("0x789".to_string(), "Market 3".to_string()),
        ];

        // Test various response formats
        let response1 = "0x123\n0x456";
        let ids1 = client.parse_market_ids(response1, &markets);
        assert_eq!(ids1.len(), 2);
        assert!(ids1.contains(&"0x123".to_string()));
        assert!(ids1.contains(&"0x456".to_string()));

        let response2 = "[ID: 0x123]\n[ID: 0x789]";
        let ids2 = client.parse_market_ids(response2, &markets);
        assert_eq!(ids2.len(), 2);
        assert!(ids2.contains(&"0x123".to_string()));
        assert!(ids2.contains(&"0x789".to_string()));

        let response3 = "Market 0x123 is compatible\nAlso 0x456";
        let ids3 = client.parse_market_ids(response3, &markets);
        assert_eq!(ids3.len(), 2);
    }
}
