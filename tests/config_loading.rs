//! Integration test: Configuration utilities
//!
//! Tests the bin_common configuration loading functionality.

use polymarket_arb_bot::bin_common::{load_config_from_env, ConfigType};
use std::env;

#[test]
fn test_strategies_config_default() {
    // Clear env var to test default
    env::remove_var("STRATEGIES_CONFIG_PATH");

    let config_path = load_config_from_env(ConfigType::Strategies);
    assert_eq!(
        config_path.to_str().unwrap(),
        "config/strategies_config.yaml"
    );
}

#[test]
fn test_bot_config_default() {
    // Clear env var to test default
    env::remove_var("CONFIG_PATH");

    let config_path = load_config_from_env(ConfigType::Bot);
    assert_eq!(config_path.to_str().unwrap(), "config.yaml");
}

#[test]
fn test_custom_config() {
    let custom = ConfigType::Custom("custom/path.yaml".to_string());
    let config_path = load_config_from_env(custom);

    assert_eq!(config_path.to_str().unwrap(), "custom/path.yaml");
}

#[test]
fn test_config_type_env_var_names() {
    assert_eq!(ConfigType::Strategies.env_var_name(), "STRATEGIES_CONFIG_PATH");
    assert_eq!(ConfigType::Bot.env_var_name(), "CONFIG_PATH");
}

#[test]
fn test_config_type_default_paths() {
    assert_eq!(
        ConfigType::Strategies.default_path(),
        "config/strategies_config.yaml"
    );
    assert_eq!(ConfigType::Bot.default_path(), "config.yaml");

    let custom = ConfigType::Custom("test.yaml".to_string());
    assert_eq!(custom.default_path(), "test.yaml");
}
