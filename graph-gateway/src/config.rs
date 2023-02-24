use crate::chains::ethereum;
use hdwallet::{self, KeyChain as _};
use indexer_selection::SecretKey;
use prelude::*;
use semver::Version;
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use std::{collections::BTreeMap, path::PathBuf};

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Respect the payment state of API keys (disable for testnets)
    pub api_key_payment_required: bool,
    pub chains: Vec<Chain>,
    /// Fisherman RPC for challenges
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub fisherman: Option<Url>,
    /// Total Gateway processes serving queries. This is used when approximating worldwide query
    /// volume.
    pub gateway_instance_count: u64,
    pub geoip_database: Option<PathBuf>,
    /// GeoIP blocked countries (ISO 3166-1 alpha-2 codes)
    #[serde(default)]
    pub geoip_blocked_countries: Vec<String>,
    /// Graph network environment identifier, inserted into Kafka messages
    pub graph_env_id: String,
    pub indexer_selection_retry_limit: usize,
    /// IPFS endpoint with access to the subgraph files
    #[serde_as(as = "DisplayFromStr")]
    pub ipfs: Url,
    /// See https://github.com/confluentinc/librdkafka/blob/master/CONFIGURATION.md
    #[serde(default)]
    pub kafka: KafkaConfig,
    /// Format log output as JSON
    pub log_json: bool,
    #[serde_as(as = "DisplayFromStr")]
    pub min_indexer_version: Version,
    #[serde_as(as = "DisplayFromStr")]
    pub network_subgraph: Url,
    pub port_api: u16,
    pub port_metrics: u16,
    pub query_budget_discount: f64,
    pub query_budget_scale: f64,
    /// Duration of API rate limiting window in seconds
    pub rate_limit_api_window_secs: u8,
    /// API rate limit per window
    pub rate_limit_api_limit: u16,
    /// Duration of IP rate limiting window in seconds
    pub rate_limit_ip_window_secs: u8,
    /// IP rate limit per window
    pub rate_limit_ip_limit: u16,
    pub restricted_deployments: Option<PathBuf>,
    /// Mnemonic for voucher signing
    #[serde_as(as = "DisplayFromStr")]
    pub signer_key: SignerKey,
    /// API keys that won't be blocked for non-payment
    #[serde(default)]
    pub special_api_keys: Vec<String>,
    /// Subgraph studio admin auth token
    pub studio_auth: String,
    /// Subgraph studio admin url
    #[serde_as(as = "DisplayFromStr")]
    pub studio_url: Url,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub subscriptions_subgraph: Option<Url>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Chain {
    pub name: String,
    #[serde_as(as = "DisplayFromStr")]
    pub rpc: Url,
    pub poll_hz: u16,
}

impl From<Chain> for ethereum::Provider {
    fn from(chain: Chain) -> Self {
        Self {
            network: chain.name,
            rpc: chain.rpc,
            block_time: Duration::from_secs(chain.poll_hz as u64),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct KafkaConfig(BTreeMap<String, String>);

impl Default for KafkaConfig {
    fn default() -> Self {
        let settings = [
            ("bootstrap.servers", ""),
            ("group.id", "graph-gateway"),
            ("message.timeout.ms", "3000"),
            ("queue.buffering.max.ms", "1000"),
            ("queue.buffering.max.messages", "100000"),
        ];
        Self(
            settings
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        )
    }
}

impl Into<rdkafka::config::ClientConfig> for KafkaConfig {
    fn into(mut self) -> rdkafka::config::ClientConfig {
        let mut settings = Self::default().0;
        settings.append(&mut self.0);

        let mut config = rdkafka::config::ClientConfig::new();
        for (k, v) in settings {
            config.set(&k, &v);
        }
        config
    }
}

pub struct SignerKey(pub SecretKey);

impl fmt::Debug for SignerKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SignerKey(..)")
    }
}

impl FromStr for SignerKey {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Wallet seed zeroized on drop
        let wallet_seed = bip39::Seed::new(
            &bip39::Mnemonic::from_phrase(s, bip39::Language::English)?,
            "",
        );
        let signer_key = hdwallet::DefaultKeyChain::new(
            hdwallet::ExtendedPrivKey::with_seed(wallet_seed.as_bytes()).expect("Invalid mnemonic"),
        )
        .derive_private_key(key_path("scalar/allocations").into())
        .expect("Failed to derive signer key")
        .0
        .private_key;
        Ok(SignerKey(
            // Convert between versions of secp256k1 lib.
            SecretKey::from_slice(signer_key.as_ref()).unwrap(),
        ))
    }
}