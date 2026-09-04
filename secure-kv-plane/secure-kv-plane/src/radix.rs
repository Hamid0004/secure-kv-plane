//! Tenant‑isolated radix tree with LRU eviction.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
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
    access_tick: Option<u64>,
}

impl RadixNode {
    pub fn new() -> Self {
        Self {
            children: HashMap::new(),
            block_id: None,
            access_tick: None,
        }
    }

    fn get_child(&self, token: u32) -> Option<Arc<RwLock<RadixNode>>> {
        self.children.get(&token).cloned()
    }

    fn insert_child(&mut self, token: u32) -> Arc<RwLock<RadixNode>> {
        let child = Arc::new(RwLock::new(RadixNode::new()));
        self.children.insert(token, child.clone());
        child
    }

    fn set_block(&mut self, block_id: String, tick: u64) {
        self.block_id = Some(block_id);
        self.access_tick = Some(tick);
    }

    fn clear_block(&mut self) -> Option<String> {
        let id = self.block_id.take();
        self.access_tick = None;
        id
    }
}

// ------------------- Tenant Cache with LRU -------------------

pub struct TenantPrefixCache {
    tenants: RwLock<HashMap<TenantId, Arc<RwLock<RadixNode>>>>,
    block_counts: RwLock<HashMap<TenantId, usize>>,
    capacity: usize,
    current_tick: AtomicU64,
}

impl TenantPrefixCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            tenants: RwLock::new(HashMap::new()),
            block_counts: RwLock::new(HashMap::new()),
            capacity,
            current_tick: AtomicU64::new(1),
        }
    }

    pub fn insert(&self, tenant_id: TenantId, tokens: Vec<u32>, block_id: String) -> Option<String> {
        let root = self.get_or_create_root(tenant_id.clone());

        let mut current = root;
        for &token in &tokens {
            let next = {
                let node = current.read();
                if let Some(child) = node.get_child(token) {
                    child
                } else {
                    drop(node);
                    let mut node_w = current.write();
                    if let Some(existing) = node_w.get_child(token) {
                        existing
                    } else {
                        node_w.insert_child(token)
                    }
                }
            };
            current = next;
        }

        let tick = self.current_tick.fetch_add(1, Ordering::Relaxed);
        {
            let mut final_node = current.write();
            final_node.set_block(block_id.clone(), tick);
        }

        self.enforce_capacity(&tenant_id)
    }

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
        let mut final_node_arc: Option<Arc<RwLock<RadixNode>>> = None;

        for (idx, &token) in tokens.iter().enumerate() {
            let node = current.read();
            if let Some(child) = node.get_child(token) {
                drop(node);
                current = child.clone();
                matched_len = idx + 1;
                let child_read = current.read();
                if let Some(id) = &child_read.block_id {
                    matched_block_id = Some(id.clone());
                    final_node_arc = Some(current.clone());
                }
            } else {
                break;
            }
        }

        if let Some(arc) = final_node_arc {
            let tick = self.current_tick.fetch_add(1, Ordering::Relaxed);
            let mut node = arc.write();
            if node.block_id.is_some() {
                node.access_tick = Some(tick);
            }
        }

        MatchResult {
            matched_len,
            matched_block_id,
        }
    }

    // ------------------- Internal Helpers -------------------

    fn get_or_create_root(&self, tenant_id: TenantId) -> Arc<RwLock<RadixNode>> {
        let mut tenants = self.tenants.write();
        if let Some(root) = tenants.get(&tenant_id) {
            root.clone()
        } else {
            let new_root = Arc::new(RwLock::new(RadixNode::new()));
            tenants.insert(tenant_id.clone(), new_root.clone());
            self.block_counts.write().insert(tenant_id, 0);
            new_root
        }
    }

    fn enforce_capacity(&self, tenant_id: &TenantId) -> Option<String> {
        let mut counts = self.block_counts.write();
        let count = counts.entry(tenant_id.clone()).or_insert(0);
        if *count < self.capacity {
            *count += 1;
            return None;
        }

        let root = {
            let tenants = self.tenants.read();
            tenants.get(tenant_id).cloned()
        };
        let root = match root {
            Some(r) => r,
            None => return None,
        };

        let (evict_node_arc, evicted_id) = self.find_lru_node(&root);
        if let Some((arc, id)) = evict_node_arc.zip(evicted_id) {
            let mut node = arc.write();
            node.clear_block();
            *count = count.saturating_sub(1);
            Some(id)
        } else {
            None
        }
    }

    fn find_lru_node(&self, root: &Arc<RwLock<RadixNode>>) -> (Option<Arc<RwLock<RadixNode>>>, Option<String>) {
        let mut best_arc = None;
        let mut best_tick = None;
        let mut best_id = None;

        self.find_lru_node_rec(root, &mut best_arc, &mut best_tick, &mut best_id);

        (best_arc, best_id)
    }

    fn find_lru_node_rec(
        &self,
        node_arc: &Arc<RwLock<RadixNode>>,
        best_arc: &mut Option<Arc<RwLock<RadixNode>>>,
        best_tick: &mut Option<u64>,
        best_id: &mut Option<String>,
    ) {
        let node = node_arc.read();

        if let Some(tick) = node.access_tick {
            if best_tick.is_none() || tick < *best_tick.as_ref().unwrap() {
                *best_tick = Some(tick);
                *best_arc = Some(node_arc.clone());
                *best_id = node.block_id.clone();
            }
        }

        for child in node.children.values() {
            self.find_lru_node_rec(child, best_arc, best_tick, best_id);
        }
    }
}

impl Default for TenantPrefixCache {
    fn default() -> Self {
        Self::new(100)
    }
}

// ------------------- Tests -------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_lru_eviction_basic() {
        let cache = TenantPrefixCache::new(2);
        let tenant = TenantId("tenant".to_string());

        let evicted = cache.insert(tenant.clone(), vec![1], "A".to_string());
        assert_eq!(evicted, None);
        let evicted = cache.insert(tenant.clone(), vec![2], "B".to_string());
        assert_eq!(evicted, None);
        let evicted = cache.insert(tenant.clone(), vec![3], "C".to_string());
        assert_eq!(evicted, Some("A".to_string()));

        let res = cache.match_prefix(&tenant, &[1]);
        assert_eq!(res.matched_block_id, None);
        let res = cache.match_prefix(&tenant, &[2]);
        assert_eq!(res.matched_block_id, Some("B".to_string()));
        let res = cache.match_prefix(&tenant, &[3]);
        assert_eq!(res.matched_block_id, Some("C".to_string()));
    }

    #[test]
    fn test_lru_eviction_with_access() {
        let cache = TenantPrefixCache::new(2);
        let tenant = TenantId("tenant".to_string());

        cache.insert(tenant.clone(), vec![1], "A".to_string());
        cache.insert(tenant.clone(), vec![2], "B".to_string());

        cache.match_prefix(&tenant, &[1]);

        let evicted = cache.insert(tenant.clone(), vec![3], "C".to_string());
        assert_eq!(evicted, Some("B".to_string()));

        let res = cache.match_prefix(&tenant, &[1]);
        assert_eq!(res.matched_block_id, Some("A".to_string()));
        let res = cache.match_prefix(&tenant, &[2]);
        assert_eq!(res.matched_block_id, None);
        let res = cache.match_prefix(&tenant, &[3]);
        assert_eq!(res.matched_block_id, Some("C".to_string()));
    }

    #[test]
    fn test_concurrent_eviction() {
        let cache = Arc::new(TenantPrefixCache::new(2));
        let tenant = TenantId("tenant".to_string());

        let mut handles = vec![];
        for i in 0..10 {
            let cache = Arc::clone(&cache);
            let tenant = tenant.clone();
            handles.push(thread::spawn(move || {
                cache.insert(tenant.clone(), vec![i], format!("B{}", i));
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let count = {
            let counts = cache.block_counts.read();
            *counts.get(&tenant).unwrap_or(&0)
        };
        assert!(count <= 2);
    }
}
