use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// A map that forgets its least recently inserted or read entry beyond `capacity`.
pub struct BoundedCache<K, V> {
    capacity: usize,
    entries: HashMap<K, V>,
    order: VecDeque<K>,
}

impl<K: Copy + Eq + Hash, V> BoundedCache<K, V> {
    pub fn new(capacity: usize) -> BoundedCache<K, V> {
        BoundedCache {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        if self.entries.contains_key(key) {
            self.touch(*key);
        }
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: K, value: V) {
        self.entries.insert(key, value);
        self.touch(key);
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn touch(&mut self, key: K) {
        self.order.retain(|existing| *existing != key);
        self.order.push_back(key);
    }
}

#[cfg(test)]
mod tests {
    use super::BoundedCache;

    #[test]
    fn least_recently_used_entries_are_evicted_first() {
        let mut cache = BoundedCache::new(2);
        cache.insert(1, "a");
        cache.insert(2, "b");
        cache.get(&1);
        cache.insert(3, "c");
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&2).is_none());
        assert!(cache.get(&1).is_some() && cache.get(&3).is_some());
    }
}
