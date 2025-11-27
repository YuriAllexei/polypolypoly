//! CLI utilities for binaries
//!
//! Handles configuration loading and environment variables
//! for all binary executables.

use std::path::PathBuf;

/// Type of configuration to load
#[derive(Debug, Clone)]
pub enum ConfigType {
    /// Sniper configuration (sniper_config.yaml)
    Sniper,
    /// Bot configuration (config.yaml)
    Bot,
    /// Custom path
    Custom(String),
}

impl ConfigType {
    /// Get the default path for this config type
    pub fn default_path(&self) -> &str {
        match self {
            ConfigType::Sniper => "config/sniper_config.yaml",
            ConfigType::Bot => "config.yaml",
            ConfigType::Custom(path) => path,
        }
    }

    /// Get the environment variable name for this config type
    pub fn env_var_name(&self) -> &str {
        match self {
            ConfigType::Sniper => "SNIPER_CONFIG_PATH",
            ConfigType::Bot => "CONFIG_PATH",
            ConfigType::Custom(_) => "CONFIG_PATH",
        }
    }
}

/// Load configuration path from environment or use default
///
/// # Arguments
/// * `config_type` - Type of configuration to load
///
/// # Returns
/// Path to the configuration file
///
/// # Examples
/// ```
/// use polymarket_bin_common::load_config_from_env;
/// use polymarket_bin_common::ConfigType;
///
/// let path = load_config_from_env(ConfigType::Sniper);
/// ```
pub fn load_config_from_env(config_type: ConfigType) -> PathBuf {
    std::env::var(config_type.env_var_name())
        .unwrap_or_else(|_| config_type.default_path().to_string())
        .into()
}

/// Parse command line arguments for a binary
///
/// Returns a vector of arguments (excluding the program name)
pub fn parse_args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_type_paths() {
        assert_eq!(ConfigType::Sniper.default_path(), "config/sniper_config.yaml");
        assert_eq!(ConfigType::Bot.default_path(), "config.yaml");
        
        let custom = ConfigType::Custom("custom/path.yaml".to_string());
        assert_eq!(custom.default_path(), "custom/path.yaml");
    }

    #[test]
    fn test_config_type_env_vars() {
        assert_eq!(ConfigType::Sniper.env_var_name(), "SNIPER_CONFIG_PATH");
        assert_eq!(ConfigType::Bot.env_var_name(), "CONFIG_PATH");
    }
}
