use bevy_ecs::prelude::Component;
use std::sync::Arc;

use crate::script::value::Value;

#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub(crate) function: usize,
    pub(crate) pc: usize,
    pub(crate) stack_base: usize,
    pub(crate) receiver: Value,
    pub(crate) locals: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ThreadState {
    Runnable,
    Queued,
    Awaiting,
    Complete,
}

#[derive(Component, Clone, Debug)]
pub(crate) struct Thread {
    pub(crate) serial: u64,
    pub(crate) frames: Vec<Frame>,
    pub(crate) stack: Vec<Value>,
    pub(crate) state: ThreadState,
}

#[derive(Clone, Debug)]
pub(crate) enum WaiterKind {
    Endon { frame: usize },
    Waittill { outputs: Vec<u32> },
    Match { values: Vec<Value> },
}

#[derive(Clone, Debug)]
pub(crate) struct Waiter {
    pub(crate) receiver: Value,
    pub(crate) name: Arc<str>,
    pub(crate) thread: u64,
    pub(crate) kind: WaiterKind,
}

/// Registration order is independent of lookup order. Handles never shift when
/// another waiter is removed, including during nested notification dispatch.
#[derive(Clone, Debug, Default)]
pub(crate) struct WaiterStore {
    next: u64,
    rows: std::collections::BTreeMap<u64, Waiter>,
    events: std::collections::BTreeMap<(u64, Arc<str>), std::collections::BTreeSet<u64>>,
    threads: std::collections::BTreeMap<u64, std::collections::BTreeSet<u64>>,
}
impl WaiterStore {
    pub fn push(&mut self, w: Waiter) {
        let id = self.next;
        self.next += 1;
        if let Value::Object(object) = w.receiver {
            self.events
                .entry((object, w.name.clone()))
                .or_default()
                .insert(id);
        }
        self.threads.entry(w.thread).or_default().insert(id);
        self.rows.insert(id, w);
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn iter(&self) -> impl Iterator<Item = &Waiter> {
        self.rows.values()
    }
    pub fn matching(
        &self,
        receiver: &Value,
        name: &Arc<str>,
        test: impl Fn(&Waiter) -> bool,
    ) -> Option<u64> {
        let Value::Object(object) = receiver else {
            return None;
        };
        self.events
            .get(&(*object, name.clone()))?
            .iter()
            .copied()
            .find(|id| self.rows.get(id).is_some_and(&test))
    }
    pub fn remove(&mut self, id: u64) -> Waiter {
        let w = self.rows.remove(&id).expect("live waiter handle");
        if let Value::Object(object) = w.receiver {
            let key = (object, w.name.clone());
            if let Some(ids) = self.events.get_mut(&key) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.events.remove(&key);
                }
            }
        }
        if let Some(ids) = self.threads.get_mut(&w.thread) {
            ids.remove(&id);
            if ids.is_empty() {
                self.threads.remove(&w.thread);
            }
        }
        w
    }
    pub fn retain_thread(&mut self, serial: u64, keep: impl Fn(&Waiter) -> bool) {
        let remove = self
            .threads
            .get(&serial)
            .into_iter()
            .flatten()
            .filter(|id| !keep(&self.rows[id]))
            .copied()
            .collect::<Vec<_>>();
        for id in remove {
            self.remove(id);
        }
    }
    pub fn remove_thread(&mut self, serial: u64) {
        self.retain_thread(serial, |_| false);
    }
    pub fn any_receiver(&self, serial: u64, test: impl Fn(&Value) -> bool) -> bool {
        self.threads
            .get(&serial)
            .into_iter()
            .flatten()
            .filter_map(|id| self.rows.get(id))
            .any(|w| test(&w.receiver))
    }
}
impl<'a> IntoIterator for &'a WaiterStore {
    type Item = &'a Waiter;
    type IntoIter = std::collections::btree_map::Values<'a, u64, Waiter>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.values()
    }
}

#[cfg(test)]
mod waiter_tests {
    use super::*;
    #[test]
    fn registrations_keep_order_when_other_threads_are_retired() {
        let mut w = WaiterStore::default();
        let name: Arc<str> = "event".into();
        for thread in [3, 1, 2] {
            w.push(Waiter {
                receiver: Value::Object(9),
                name: name.clone(),
                thread,
                kind: WaiterKind::Endon { frame: 0 },
            });
        }
        let id = w.matching(&Value::Object(9), &name, |_| true).unwrap();
        assert_eq!(w.remove(id).thread, 3);
        w.remove_thread(1);
        let id = w.matching(&Value::Object(9), &name, |_| true).unwrap();
        assert_eq!(w.remove(id).thread, 2);
        assert_eq!(w.len(), 0);
        assert!(w.events.is_empty() && w.threads.is_empty());
    }
}
