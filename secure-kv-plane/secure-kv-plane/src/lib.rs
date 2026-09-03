//! Tenant‑isolated KV cache core for multi‑tenant LLM inference.
//! Uses HMAC‑SHA256 to salt cache keys per tenant.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

// ------------------- Core Types -------------------

/// Identifier for a tenant (customer).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantId(pub String);

/// A sequence of token IDs (the prompt prefix that would be cached).
#[derive(Debug, Clone)]
pub struct PrefixTokens(pub Vec<u32>);

/// A mock KV block – in reality this would hold tensors or byte buffers.
#[derive(Debug, Clone)]
pub struct KvBlock {
    pub data: Vec<u8>,          // placeholder for actual KV data
    pub shape: (usize, usize),  // e.g., (num_heads, head_dim)
}

// ------------------- Key Manager (The Defense) -------------------

/// Derives tenant‑specific cache keys using HMAC‑SHA256.
pub struct KeyManager {
    tenant_secret: Vec<u8>,
}

impl KeyManager {
    /// Create a new KeyManager with a secret shared only with the tenant.
    pub fn new(tenant_secret: impl Into<Vec<u8>>) -> Self {
        Self {
            tenant_secret: tenant_secret.into(),
        }
    }

    /// Derive a 256‑bit cache key for a given tenant and token sequence.
    ///
    /// The key is:
    ///     HMAC(tenant_secret, tenant_id ‖ SHA256(tokens))
    ///
    /// This ensures that different tenants get completely different keys,
    /// even when the token sequence is identical.
    pub fn derive_cache_key(&self, tenant_id: &TenantId, tokens: &PrefixTokens) -> [u8; 32] {
        // 1. Compute hash of the token sequence.
        let token_hash = self.hash_tokens(tokens);

        // 2. Build the message: tenant_id bytes + token_hash.
        let mut message = Vec::with_capacity(tenant_id.0.len() + 32);
        message.extend_from_slice(tenant_id.0.as_bytes());
        message.extend_from_slice(&token_hash);

        // 3. Compute HMAC‑SHA256 using the tenant_secret.
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.tenant_secret)
                .expect("HMAC can take any key length");
        mac.update(&message);
        let result = mac.finalize();

        // 4. Return the 32‑byte array.
        result.into_bytes().into()
    }

    /// Internal helper: SHA‑256 hash of token IDs.
    fn hash_tokens(&self, tokens: &PrefixTokens) -> [u8; 32] {
        let mut hasher = Sha256::new();
        // We hash each token as u32 in little‑endian, concatenated.
        for token in &tokens.0 {
            hasher.update(&token.to_le_bytes());
        }
        let hash = hasher.finalize();
        hash.into()
    }
}

// ------------------- Block Store (In‑Memory) -------------------

/// A tenant‑isolated store that maps derived cache keys to KV blocks.
pub struct TenantIsolatedStore {
    store: HashMap<[u8; 32], KvBlock>,
}

impl TenantIsolatedStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }

    /// Store a KV block for a specific tenant and token prefix.
    pub fn store(
        &mut self,
        key_manager: &KeyManager,
        tenant_id: &TenantId,
        tokens: &PrefixTokens,
        block: KvBlock,
    ) {
        let key = key_manager.derive_cache_key(tenant_id, tokens);
        self.store.insert(key, block);
    }

    /// Look up a KV block for a specific tenant and token prefix.
    /// Returns `None` if the block is not found (or belongs to a different tenant).
    pub fn lookup(
        &self,
        key_manager: &KeyManager,
        tenant_id: &TenantId,
        tokens: &PrefixTokens,
    ) -> Option<&KvBlock> {
        let key = key_manager.derive_cache_key(tenant_id, tokens);
        self.store.get(&key)
    }
}

impl Default for TenantIsolatedStore {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------- Tests -------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    /// Generate a random 32‑byte secret for testing.
    fn random_secret() -> [u8; 32] {
        rand::thread_rng().gen()
    }

    #[test]
    fn test_tenant_a_and_b_different_keys_same_tokens() {
        let secret_a = random_secret();
        let secret_b = random_secret(); // different secret

        let km_a = KeyManager::new(secret_a);
        let km_b = KeyManager::new(secret_b);

        let tenant_a = TenantId("tenant-a".to_string());
        let tenant_b = TenantId("tenant-b".to_string());

        let tokens = PrefixTokens(vec![101, 202, 303, 404]); // dummy token IDs

        let key_a = km_a.derive_cache_key(&tenant_a, &tokens);
        let key_b = km_b.derive_cache_key(&tenant_b, &tokens);

        assert_ne!(
            key_a, key_b,
            "Tenant A and B must produce different keys for the same tokens"
        );
    }

    #[test]
    fn test_same_tenant_same_tokens_same_key() {
        let secret = random_secret();
        let km = KeyManager::new(secret);
        let tenant = TenantId("tenant-x".to_string());
        let tokens = PrefixTokens(vec![1, 2, 3]);

        let key1 = km.derive_cache_key(&tenant, &tokens);
        let key2 = km.derive_cache_key(&tenant, &tokens);

        assert_eq!(key1, key2, "Repeated derivation must yield the same key");
    }

    #[test]
    fn test_tenant_b_cannot_lookup_tenant_a_block() {
        let secret_a = random_secret();
        let secret_b = random_secret();

        let km_a = KeyManager::new(secret_a);
        let km_b = KeyManager::new(secret_b);

        let tenant_a = TenantId("tenant-a".to_string());
        let tenant_b = TenantId("tenant-b".to_string());

        let tokens = PrefixTokens(vec![42, 99]);

        let block = KvBlock {
            data: vec![0xAA, 0xBB, 0xCC],
            shape: (2, 64),
        };

        // Store using tenant A
        let mut store = TenantIsolatedStore::new();
        store.store(&km_a, &tenant_a, &tokens, block);

        // Try to lookup with tenant B (different secret and ID)
        let result = store.lookup(&km_b, &tenant_b, &tokens);

        assert!(
            result.is_none(),
            "Tenant B should NOT see Tenant A's cache block"
        );
    }

    #[test]
    fn test_tenant_a_can_lookup_own_block() {
        let secret = random_secret();
        let km = KeyManager::new(secret);
        let tenant = TenantId("tenant-a".to_string());
        let tokens = PrefixTokens(vec![5, 6, 7]);

        let block = KvBlock {
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
            shape: (8, 128),
        };

        let mut store = TenantIsolatedStore::new();
        store.store(&km, &tenant, &tokens, block.clone());

        let lookup = store.lookup(&km, &tenant, &tokens);

        assert!(lookup.is_some(), "Tenant A must retrieve its own block");
        assert_eq!(lookup.unwrap().data, block.data);
        assert_eq!(lookup.unwrap().shape, block.shape);
    }

    #[test]
    fn test_different_tokens_same_tenant_different_keys() {
        let secret = random_secret();
        let km = KeyManager::new(secret);
        let tenant = TenantId("tenant-a".to_string());

        let tokens1 = PrefixTokens(vec![10, 20]);
        let tokens2 = PrefixTokens(vec![10, 21]); // only last token differs

        let key1 = km.derive_cache_key(&tenant, &tokens1);
        let key2 = km.derive_cache_key(&tenant, &tokens2);

        assert_ne!(key1, key2, "Different token sequences must yield different keys");
    }
}