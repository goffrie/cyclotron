use serde_json;
use std::time::Duration;
use std::collections::{
    HashMap,
    HashSet,
};

// Copied from dropbox/cyclotron/src/event.rs

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub struct SpanId(pub u64);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum AsyncOutcome {
    Success,
    Cancelled,
    Error(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum TraceEvent {
    /// Logged the first time a future is polled after a logger is installed.  If this is the first
    /// time the future is *ever* polled, `is_restart` will be false.
    AsyncStart {
        name: String,
        id: SpanId,
        parent_id: SpanId,
        ts: Duration,
        metadata: serde_json::Value,
        is_restart: bool,
    },
    /// Logged immediately before each time the future is polled
    AsyncOnCPU { id: SpanId, ts: Duration },
    /// Logged immediately after each time the future is polled
    AsyncOffCPU { id: SpanId, ts: Duration },
    /// Logged when the future is completed. Returning `Ok(Async::Ready(..))` will set
    /// `AsyncOutcome::Success`, `Err(e)` will set `AsyncOutcome::Error`, and dropping the future
    /// will set `AsyncOutcome::Cancelled`.
    AsyncEnd {
        id: SpanId,
        ts: Duration,
        outcome: AsyncOutcome,
    },

    /// Logged when a sync span is entered.  Note that since we don't repeatedly
    /// poll synchronous spans, we don't make an attempt to restart them when
    /// the logger changes.
    SyncStart {
        name: String,
        id: SpanId,
        parent_id: SpanId,
        ts: Duration,
        metadata: serde_json::Value,
    },
    /// Logged when a sync span is exited and the current generation matches the
    /// one at the span's start.
    SyncEnd { id: SpanId, ts: Duration },

    /// Logged when a logger is installed on a thread.  If this corresponds with thread creation,
    /// `is_restart` will be set to false.
    ThreadStart {
        name: String,
        id: SpanId,
        ts: Duration,
        is_restart: bool,
    },
    /// Logged when a thread is dropped.
    ThreadEnd { id: SpanId, ts: Duration },

    /// Logged when a wakeup originates from a traced thread, noting the current span and span that's
    /// being woken up
    Wakeup {
        waking_span: SpanId,
        parked_span: SpanId,
        ts: Duration,
    },
}

#[derive(Clone, Eq, Hash)]
struct EventResult {
    buf: String, // buffer before json conversion; list includes e.g. both AsyncStart and AsyncEnd
    ts: Duration, // the ts from self.event, extracted for convenient sorting
}

// Allow sorting by timestamp.
impl Ord for EventResult {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering { self.ts.cmp(&other.ts) }
}
impl PartialOrd for EventResult {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.ts.cmp(&other.ts)) }
}
impl PartialEq for EventResult {
    fn eq(&self, other: &Self) -> bool { self.ts == other.ts }
}

#[derive(Clone)]
struct EventNode {
    events: Vec<EventResult>,
    name: String,
    parent: Option<SpanId>,
    children: Vec<SpanId>,
}

#[derive(Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Wakeup {
    event: EventResult,
    waking_span: SpanId,
    parked_span: SpanId,
}

pub struct EventTree {
    slab: HashMap<SpanId, EventNode>,
    roots: HashSet<SpanId>,
    // emit these in postprocessing, if both nodes are in the tree
    wakeups: HashSet<Wakeup>,
    // what we're gonna filter for
    goal_names: HashSet<String>,
    goal_spans: HashSet<SpanId>,
    // filter out any wakeups originating from this node (popular choice: Control)
    hide_wakeups_from_names: HashSet<String>,
    hide_wakeups_from_spans: HashSet<SpanId>,
}

impl EventTree {
    #[cfg(test)]
    pub fn new(goals: Vec<String>) -> Self {
        EventTree::new_hide_wakeups(goals, vec!["Control".to_string()])
    }

    pub fn new_hide_wakeups(goals: Vec<String>, hide_wakeups_from: Vec<String>) -> Self {
        EventTree {
            slab: HashMap::new(),
            roots: HashSet::new(),
            wakeups: HashSet::new(),
            goal_names: goals.into_iter().collect(),
            goal_spans: HashSet::new(),
            hide_wakeups_from_names: hide_wakeups_from.into_iter().collect(),
            hide_wakeups_from_spans: HashSet::new(),
        }
    }

    fn add_node(&mut self, id: SpanId, buf: String, name: String, ts: Duration, parent: Option<SpanId>) -> Result<(), (failure::Error, String)> {
        if self.slab.contains_key(&id) {
            return Err((failure::format_err!("duplicate node"), buf));
        }
        if self.goal_names.contains(&name) || self.goal_names.is_empty() {
            self.goal_spans.insert(id);
        }
        if self.hide_wakeups_from_names.contains(&name) {
            self.hide_wakeups_from_spans.insert(id);
        }
        self.slab.insert(id, EventNode {
            events: vec![EventResult { buf, ts }],
            name,
            parent,
            children: vec![],
        });
        Ok(())
    }

    pub fn add(&mut self, buf: String) -> Result<(), (failure::Error, String)> {
        let event: TraceEvent = match serde_json::from_str(&buf) {
            Ok(event) => event,
            Err(e) => return Err((e.into(), buf)),
        };
        match event {
            // Add new root.
            TraceEvent::ThreadStart { id, name, ts, .. } => {
                self.add_node(id, buf, name, ts, None)?;
                self.roots.insert(id);
            }

            // Add new node with a parent.
            TraceEvent::AsyncStart { id, parent_id, name, ts, .. }
            | TraceEvent::SyncStart { id, parent_id, name, ts, .. } => {
                assert!(!self.slab.contains_key(&id), "duplicate node");
                if let Some(parent_node) = self.slab.get_mut(&parent_id) {
                    parent_node.children.push(id);
                    self.add_node(id, buf, name, ts, Some(parent_id))?;
                } else {
                    println!("warning: parentless node {:?} (alleged parent: {:?}); treating as root", id, parent_id);
                    self.add_node(id, buf, name, ts, None)?;
                    self.roots.insert(id);
                }
            },

            // Add event to existing node in the tree.
            TraceEvent::AsyncOnCPU { id, ts, .. }
            | TraceEvent::AsyncOffCPU { id, ts, .. }
            | TraceEvent::AsyncEnd { id, ts, .. }
            | TraceEvent::SyncEnd { id, ts, .. }
            | TraceEvent::ThreadEnd { id, ts, .. } => {
                let node = self.slab.get_mut(&id).expect("nodeless event");
                node.events.push(EventResult { buf, ts });
            }

            // Add new wakeup.
            TraceEvent::Wakeup { waking_span, parked_span, ts, .. } => {
                self.wakeups.insert(Wakeup { event: EventResult { buf, ts }, waking_span, parked_span });
            }
        }
        Ok(())
    }

    // TODO: be able to do this filter in-line
    // Guaranteed to return in root-first order (parents before children), and wakeups last, i guess.
    pub fn filter(&self) -> Vec<String> {
        let mut seen_ids = HashSet::new();
        let mut result = vec![];
        for id in &self.goal_spans {
            let node = self.slab.get(id).expect("this node missing during filter");
            // Process this node's ancestors.
            self.add_ancestors(&mut seen_ids, &mut result, node.parent);
            // Add all its children, and children's children, and so on.
            // NB this includes adding the node itself
            self.add_children(&mut seen_ids, &mut result, *id);
        }
        for wakeup in &self.wakeups {
            // Add wakeup only if both of its endpoints are included in the result.
            if seen_ids.contains(&wakeup.waking_span) && seen_ids.contains(&wakeup.parked_span) {
                // (and if we weren't told explicitly to avoid printing it)
                if !self.hide_wakeups_from_spans.contains(&wakeup.waking_span) {
                    // println!("adding wakeup: {}", wakeup.event.buf);
                    result.push(wakeup.event.clone());
                }
            }
        }
        result.sort();
        result.into_iter().map(|x| x.buf).collect()
    }

    fn add_ancestors(&self, seen_ids: &mut HashSet<SpanId>, result: &mut Vec<EventResult>, ancestor_id: Option<SpanId>) {
        if let Some(id) = ancestor_id {
            if !seen_ids.contains(&id) {
                let node = self.slab.get(&id).expect("ancestor node missing");
                //println!("adding {} events from node named '{}'", node.events.len(), node.name);
                seen_ids.insert(id);
                // Add after iterating, to ensure parent-first order.
                self.add_ancestors(seen_ids, result, node.parent);
                for event in &node.events {
                    result.push(event.clone());
                }
            }
        }
    }

    fn add_children(&self, seen_ids: &mut HashSet<SpanId>, result: &mut Vec<EventResult>, id: SpanId) {
        if !seen_ids.contains(&id) {
            // Add before iterating, to ensure parent-first order.
            let node = self.slab.get(&id).expect("child node missing");
            //println!("adding {} events from node named '{}'", node.events.len(), node.name);
            seen_ids.insert(id);
            for event in &node.events {
                result.push(event.clone());
            }
            for child in &node.children {
                self.add_children(seen_ids, result, *child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EventTree;

    fn buf_thread_start(name: &str, id: usize) -> String {
        format!("{{\"ThreadStart\":{{\"name\":\"{}\",\"id\":{},\"ts\":{{\"secs\":0,\"nanos\":0}},\"is_restart\":false}}}}", name, id)
    }

    fn buf_sync_start(name: &str, id: usize, parent_id: usize) -> String {
        format!("{{\"SyncStart\":{{\"name\":\"{}\",\"id\":{},\"parent_id\":{},\"ts\":{{\"secs\":0,\"nanos\":0}},\"metadata\":null}}}}", name, id, parent_id)
    }

    fn buf_sync_end(id: usize) -> String {
        format!("{{\"SyncEnd\":{{\"id\":{},\"ts\":{{\"secs\":0,\"nanos\":0}}}}}}", id)
    }

    fn buf_wakeup(waking_id: usize, parked_id: usize, ts: usize) -> String {
        format!("{{\"Wakeup\":{{\"waking_span\":{},\"parked_span\":{},\"ts\":{{\"secs\":0,\"nanos\":{}}}}}}}", waking_id, parked_id, ts)
    }

    #[test]
    fn test_event_tree_multiple_roots() {
        let mut tree = EventTree::new(vec![]);
        let mut root_id = 0;
        for name in &["John", "Paul", "George", "Ringo"] {
            tree.add(buf_thread_start(name, root_id)).expect("add");
            root_id += 1;
        }
        assert_eq!(tree.roots.len(), 4);
    }

    #[test]
    fn test_event_tree_no_goals_no_problem() {
        let mut tree = EventTree::new(vec![]);
        let mut root_id = 0;
        for name in &["John", "Paul", "George", "Ringo"] {
            tree.add(buf_thread_start(name, root_id)).expect("add");
            root_id += 1;
        }
        assert_eq!(tree.filter().len(), 4);
    }

    #[test]
    fn test_event_child_basic() {
        let mut tree = EventTree::new(vec!["Graydon".to_string()]);
        tree.add(buf_thread_start("Graydon", 0)).expect("add root");
        tree.add(buf_sync_start("Niko", 1, 0)).expect("add child");
        tree.add(buf_sync_start("Patrick", 2, 0)).expect("add child");
        assert_eq!(tree.filter().len(), 3);
    }

    #[test]
    fn test_event_parent_basic() {
        let mut tree = EventTree::new(vec!["Niko".to_string()]);
        tree.add(buf_thread_start("Graydon", 0)).expect("add root");
        tree.add(buf_sync_start("Niko", 1, 0)).expect("add child");
        tree.add(buf_sync_start("Patrick", 2, 0)).expect("add child");
        assert_eq!(tree.filter().len(), 2); // not include patrick
    }

    #[test]
    fn test_event_not_include_duplicates() {
        let mut tree = EventTree::new(vec!["Niko".to_string(), "Patrick".to_string()]);
        tree.add(buf_thread_start("Graydon", 0)).expect("add root");
        tree.add(buf_sync_start("Niko", 1, 0)).expect("add child");
        tree.add(buf_sync_start("Patrick", 2, 0)).expect("add child");
        assert_eq!(tree.filter().len(), 3);
    }

    #[test]
    fn test_event_include_end_span() {
        let mut tree = EventTree::new(vec!["Niko".to_string()]);
        tree.add(buf_thread_start("Graydon", 0)).expect("add root");
        tree.add(buf_sync_start("Niko", 1, 0)).expect("add child");
        tree.add(buf_sync_start("Patrick", 2, 0)).expect("add child");
        tree.add(buf_sync_end(2)).expect("add child");
        tree.add(buf_sync_end(1)).expect("add child");
        assert_eq!(tree.filter().len(), 3);
    }

    #[test]
    fn test_event_wakeups() {
        let mut tree = EventTree::new_hide_wakeups(vec!["Niko".to_string()], vec!["Graydon".to_string()]);
        tree.add(buf_thread_start("Graydon", 0)).expect("add root");
        tree.add(buf_sync_start("Niko", 1, 0)).expect("add child");
        tree.add(buf_sync_start("Patrick", 2, 0)).expect("add child");
        // include these
        for ts in 0..20 {
            tree.add(buf_wakeup(1, 0, ts)).expect("add wakeup");
        }
        // don't include these - patrick not in goals
        for ts in 0..40 {
            tree.add(buf_wakeup(1, 2, ts)).expect("add wakeup");
        }
        // don't include these - hiding wakeup from graydon
        for ts in 0..80 {
            tree.add(buf_wakeup(0, 1, ts)).expect("add wakeup");
        }
        assert_eq!(tree.filter().len(), 22);
    }
}
