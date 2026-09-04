//! Tenant‑isolated radix tree (trie) for efficient prefix matching.

use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::collections::HashMap;
use std::sync::Arc;

// ------------------- Types -------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantId(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    pub matched_len: usize,
    pub matched_block_id: Option<String>,
}

// ------------------- Radix Node -------------------

#[derive(Default)]
pub struct RadixNode {
    children: HashMap<u32, Arc<RwLock<RadixNode>>>,
    block_id: Option<String>,
}

impl RadixNode {
    pub fn new() -> Self {
        Self {
            children: HashMap::new(),
            block_id: None,
        }
    }

    // Helper to get a child by token, returning an Arc clone.
    fn get_child(&self, token: u32) -> Option<Arc<RwLock<RadixNode>>> {
        self.children.get(&token).cloned()
    }

    // Helper to insert a child (returns the new child's Arc).
    fn insert_child(&mut self, token: u32) -> Arc<RwLock<RadixNode>> {
        let child = Arc::new(RwLock::new(RadixNode::new()));
        self.children.insert(token, child.clone());
        child
    }
}

// ------------------- Tenant Cache -------------------

pub struct TenantPrefixCache {
    tenants: RwLock<HashMap<TenantId, Arc<RwLock<RadixNode>>>>,
}

impl TenantPrefixCache {
    pub fn new() -> Self {
        Self {
            tenants: RwLock::new(HashMap::new()),
        }
    }

    /// Insert a block for a tenant.
    pub fn insert(&self, tenant_id: TenantId, tokens: Vec<u32>, block_id: String) {
        let root = self.get_or_create_root(tenant_id);

        let mut current = root;
        for &token in &tokens {
            // Try to get child under read lock.
            let next = {
                let node = current.read();
                if let Some(child) = node.get_child(token) {
                    child
                } else {
                    // Need to insert – upgrade to write lock.
                    drop(node);
                    let mut node_w = current.write();
                    // Double-check (could have been inserted by another thread).
                    if let Some(existing) = node_w.get_child(token) {
                        existing
                    } else {
                        node_w.insert_child(token)
                    }
                }
            };
            current = next;
        }

        // At final node – set block_id.
        let mut final_node = current.write();
        final_node.block_id = Some(block_id);
    }

    /// Match longest prefix.
    pub fn match_prefix(&self, tenant_id: &TenantId, tokens: &[u32]) -> MatchResult {
        let root = {
            let tenants = self.tenants.read();
            tenants.get(tenant_id).cloned()
        };
        let root = match root {
            Some(r) => r,
            None => return MatchResult { matched_len: 0, matched_block_id: None },
        };

        let mut current = root;
        let mut matched_len = 0;
        let mut matched_block_id = None;

        for (idx, &token) in tokens.iter().enumerate() {
            let node = current.read();
            if let Some(child) = node.get_child(token) {
                // Move to child.
                drop(node);
                current = child;
                matched_len = idx + 1;
                // Check if this node has a block_id.
                let child_node = current.read();
                if let Some(id) = &child_node.block_id {
                    matched_block_id = Some(id.clone());
                }
                // Continue; we drop the guard at the end of iteration.
            } else {
                // No child – stop matching.
                break;
            }
        }

        MatchResult {
            matched_len,
            matched_block_id,
        }
    }

    // Helper: get or create tenant root.
    fn get_or_create_root(&self, tenant_id: TenantId) -> Arc<RwLock<RadixNode>> {
        let mut tenants = self.tenants.write();
        if let Some(root) = tenants.get(&tenant_id) {
            root.clone()
        } else {
            let new_root = Arc::new(RwLock::new(RadixNode::new()));
            tenants.insert(tenant_id, new_root.clone());
            new_root
        }
    }
}

impl Default for TenantPrefixCache {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------- Tests -------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_insert_and_match() {
        let cache = TenantPrefixCache::new();
        let tenant = TenantId("tenant-a".to_string());

        cache.insert(tenant.clone(), vec![1, 2, 3, 4], "B1".to_string());
        cache.insert(tenant.clone(), vec![1, 2, 3, 5], "B2".to_string());

        // Query partial match – should stop at 3 (no block_id at 1,2,3)
        let res = cache.match_prefix(&tenant, &[1, 2, 3, 6]);
        assert_eq!(res.matched_len, 3);
        assert_eq!(res.matched_block_id, None);

        // Query extending beyond B1
        let res = cache.match_prefix(&tenant, &[1, 2, 3, 4, 9]);
        assert_eq!(res.matched_len, 4);
        assert_eq!(res.matched_block_id, Some("B1".to_string()));

        // Exact match
        let res = cache.match_prefix(&tenant, &[1, 2, 3, 4]);
        assert_eq!(res.matched_len, 4);
        assert_eq!(res.matched_block_id, Some("B1".to_string()));
    }

    #[test]
    fn test_tenant_isolation() {
        let cache = TenantPrefixCache::new();
        let tenant_a = TenantId("tenant-a".to_string());
        let tenant_b = TenantId("tenant-b".to_string());

        cache.insert(tenant_a.clone(), vec![10, 20, 30], "A-block".to_string());
        cache.insert(tenant_b.clone(), vec![10, 20, 30], "B-block".to_string());

        let res_a = cache.match_prefix(&tenant_a, &[10, 20, 30, 40]);
        assert_eq!(res_a.matched_block_id, Some("A-block".to_string()));

        let res_b = cache.match_prefix(&tenant_b, &[10, 20, 30, 40]);
        assert_eq!(res_b.matched_block_id, Some("B-block".to_string()));

        // Tenant B can't see A's block.
        let res_b_again = cache.match_prefix(&tenant_b, &[10, 20, 30]);
        assert_eq!(res_b_again.matched_block_id, Some("B-block".to_string()));
    }

    #[test]
    fn test_concurrent_reading_and_writing() {
        let cache = Arc::new(TenantPrefixCache::new());
        let tenant = TenantId("tenant-x".to_string());

        let mut handles = vec![];
        // Spawn 10 readers.
        for _ in 0..10 {
            let cache = Arc::clone(&cache);
            let tenant = tenant.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..50 {
                    let _ = cache.match_prefix(&tenant, &[1, 2, 3]);
                }
            }));
        }

        // Spawn one writer.
        let writer_cache = Arc::clone(&cache);
        let writer_tenant = tenant.clone();
        handles.push(thread::spawn(move || {
            writer_cache.insert(writer_tenant, vec![1, 2, 3, 4], "B1".to_string());
        }));

        for h in handles {
            h.join().unwrap();
        }

        let res = cache.match_prefix(&tenant, &[1, 2, 3, 4, 5]);
        assert_eq!(res.matched_len, 4);
        assert_eq!(res.matched_block_id, Some("B1".to_string()));
    }
}