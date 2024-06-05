use std::{collections::HashMap, sync::Arc, time::SystemTime};

use alloy_primitives::{Address, U256};
use alloy_sol_types::Eip712Domain;
use ethers::signers::Wallet;
use rand::RngCore;
pub use receipts::{QueryStatus as ReceiptStatus, ReceiptPool};
use secp256k1::SecretKey;
use tap_core::{receipt::Receipt, signed_message::EIP712SignedMessage};
use thegraph_core::types::DeploymentId;
use tokio::sync::{Mutex, RwLock};

#[allow(clippy::type_complexity)]
pub struct ReceiptSigner {
    signer: SecretKey,
    domain: Eip712Domain,
    allocations: RwLock<HashMap<(Address, DeploymentId), Address>>,
    legacy_signer: &'static SecretKey,
    legacy_pools: RwLock<HashMap<(Address, DeploymentId), Arc<Mutex<ReceiptPool>>>>,
}

pub enum ScalarReceipt {
    Legacy(u128, Vec<u8>),
    TAP(EIP712SignedMessage<Receipt>),
}

impl ScalarReceipt {
    pub fn grt_value(&self) -> u128 {
        match self {
            ScalarReceipt::Legacy(value, _) => *value,
            ScalarReceipt::TAP(receipt) => receipt.message.value,
        }
    }

    pub fn allocation(&self) -> Address {
        match self {
            ScalarReceipt::Legacy(_, receipt) => Address::from_slice(&receipt[0..20]),
            ScalarReceipt::TAP(receipt) => receipt.message.allocation_id,
        }
    }

    pub fn serialize(&self) -> String {
        match self {
            ScalarReceipt::Legacy(_, receipt) => hex::encode(&receipt[..(receipt.len() - 32)]),
            ScalarReceipt::TAP(receipt) => serde_json::to_string(&receipt).unwrap(),
        }
    }

    pub fn header_name(&self) -> &'static str {
        match self {
            ScalarReceipt::Legacy(_, _) => "Scalar-Receipt",
            ScalarReceipt::TAP(_) => "Tap-Receipt",
        }
    }
}

impl ReceiptSigner {
    pub async fn new(
        signer: SecretKey,
        chain_id: U256,
        verifier: Address,
        legacy_signer: &'static SecretKey,
    ) -> Self {
        Self {
            signer,
            domain: Eip712Domain {
                name: Some("TAP".into()),
                version: Some("1".into()),
                chain_id: Some(chain_id),
                verifying_contract: Some(verifier),
                salt: None,
            },
            allocations: RwLock::default(),
            legacy_signer,
            legacy_pools: RwLock::default(),
        }
    }

    pub async fn create_receipt(
        &self,
        indexer: Address,
        deployment: DeploymentId,
        fee: u128,
    ) -> Option<ScalarReceipt> {
        let allocation = *self.allocations.read().await.get(&(indexer, deployment))?;
        // Nonce generated with CSPRNG (ChaCha12), to avoid collisison with receipts generated by
        // other gateway processes.
        // See https://docs.rs/rand/latest/rand/rngs/index.html#our-generators.
        let nonce = rand::thread_rng().next_u64();
        let timestamp_ns = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .try_into()
            .unwrap();
        let receipt = Receipt {
            allocation_id: allocation.0 .0.into(),
            timestamp_ns,
            nonce,
            value: fee,
        };
        let wallet =
            Wallet::from_bytes(self.signer.as_ref()).expect("failed to prepare receipt wallet");
        let signed = EIP712SignedMessage::new(&self.domain, receipt, &wallet)
            .expect("failed to sign receipt");
        Some(ScalarReceipt::TAP(signed))
    }

    pub async fn create_legacy_receipt(
        &self,
        indexer: Address,
        deployment: DeploymentId,
        fee: u128,
    ) -> Option<ScalarReceipt> {
        let legacy_pool = self
            .legacy_pools
            .read()
            .await
            .get(&(indexer, deployment))?
            .clone();
        let mut legacy_pool = legacy_pool.lock().await;
        let receipt = legacy_pool.commit(self.legacy_signer, fee.into()).ok()?;
        Some(ScalarReceipt::Legacy(fee, receipt))
    }

    pub async fn record_receipt(
        &self,
        indexer: Address,
        deployment: DeploymentId,
        receipt: &ScalarReceipt,
        status: ReceiptStatus,
    ) {
        if let ScalarReceipt::Legacy(_, receipt) = receipt {
            let legacy_pool = self.legacy_pools.read().await;
            let mut legacy_pool = match legacy_pool.get(&(indexer, deployment)) {
                Some(legacy_pool) => legacy_pool.lock().await,
                None => return,
            };
            legacy_pool.release(receipt, status);
        }
    }

    pub async fn update_allocations(&self, indexings: &HashMap<(Address, DeploymentId), Address>) {
        // refresh legacy pools
        {
            let mut legacy_pools = self.legacy_pools.write().await;
            legacy_pools.retain(|indexing, _| indexings.contains_key(indexing));
            for (indexing, allocation) in indexings {
                legacy_pools
                    .entry(*indexing)
                    .or_insert_with(|| Arc::new(Mutex::new(ReceiptPool::new(allocation.0 .0))));
            }
        }

        // refresh allocations
        let mut allocations = self.allocations.write().await;
        allocations.retain(|k, _| indexings.contains_key(k));
        for (indexing, allocation) in indexings {
            allocations.insert(*indexing, *allocation);
        }
    }
}
