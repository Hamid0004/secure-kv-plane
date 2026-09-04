use parking_lot::RwLock;
use pyo3::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

// ------------------- Module Declarations -------------------
pub mod radix;
use radix::{TenantId, TenantPrefixCache};

// ------------------- KeyManager (HMAC) -------------------
// ... (previous HMAC code)
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct KeyManager {
    tenant_secret: Vec<u8>,
}

impl KeyManager {
    pub fn new(secret: Vec<u8>) -> Self {
        Self { tenant_secret: secret }
    }

    pub fn derive_key(&self, tenant_id: &str, tokens: &[u32]) -> String {
        let mut hasher = Sha256::new();
        for t in tokens {
            hasher.update(&t.to_le_bytes());
        }
        let hash = hasher.finalize();

        let mut msg = Vec::with_capacity(tenant_id.len() + 32);
        msg.extend_from_slice(tenant_id.as_bytes());
        msg.extend_from_slice(&hash);

        let mut mac = Hmac::<Sha256>::new_from_slice(&self.tenant_secret).unwrap();
        mac.update(&msg);
        hex::encode(mac.finalize().into_bytes())
    }
}

// ------------------- Python Wrapper -------------------

#[pyclass]
struct PySecureCache {
    inner: Arc<RwLock<TenantPrefixCache>>,
    key_manager: KeyManager,
}

#[pymethods]
impl PySecureCache {
    #[new]
    fn new(secret: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(TenantPrefixCache::new())),
            key_manager: KeyManager::new(secret),
        }
    }

    fn insert(&mut self, tenant_id: String, tokens: Vec<u32>, block_data: Vec<u8>) {
        // Generate the HMAC-based block ID.
        let block_id = self.key_manager.derive_key(&tenant_id, &tokens);
        
        // Lock and insert into the radix tree.
        let mut cache = self.inner.write();
        let tenant = TenantId(tenant_id.clone());
        cache.insert(tenant, tokens, block_id);
    }

    fn match_prefix(&self, tenant_id: String, tokens: Vec<u32>) -> (usize, Option<String>) {
        let cache = self.inner.read();
        let tenant = TenantId(tenant_id);
        let result = cache.match_prefix(&tenant, &tokens);
        (result.matched_len, result.matched_block_id)
    }

    fn lookup_block(&self, block_id: String) -> Vec<u8> {
        // Placeholder: In a real system, you'd fetch from a KV store.
        // For now, return the block_id as bytes to simulate.
        block_id.into_bytes()
    }
}

#[pymodule]
fn secure_kv_plane(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySecureCache>()?;
    Ok(())
}