use std::time::Duration;
use serde_json;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub struct SpanId(pub u64);

#[derive(Debug, Deserialize, Serialize)]
pub enum AsyncOutcome {
    Success,
    Cancelled,
    Error(String),
}

#[derive(Debug, Deserialize, Serialize)]
pub enum TraceEvent {
    AsyncStart {
        name: String,
        id: SpanId,
        parent_id: SpanId,
        ts: Duration,
        metadata: serde_json::Value,
    },
    AsyncOnCPU {
        id: SpanId,
        ts: Duration,
    },
    AsyncOffCPU {
        id: SpanId,
        ts: Duration,
    },
    AsyncEnd {
        id: SpanId,
        ts: Duration,
        outcome: AsyncOutcome,
    },

    SyncStart {
        name: String,
        id: SpanId,
        parent_id: SpanId,
        ts: Duration,
        metadata: serde_json::Value,
    },
    SyncEnd {
        id: SpanId,
        ts: Duration,
    },

    ThreadStart {
        name: String,
        id: SpanId,
        ts: Duration,
    },
    ThreadEnd {
        id: SpanId,
        ts: Duration,
    },

    Wakeup {
        waking_span: SpanId,
        parked_span: SpanId,
        ts: Duration,
    },
}

impl TraceEvent {
    pub fn ts(&self) -> Duration {
        use self::TraceEvent::*;
        match *self {
            AsyncStart { ts, .. }
            | AsyncOnCPU { ts, .. }
            | AsyncOffCPU { ts, .. }
            | AsyncEnd { ts, .. }
            | SyncStart { ts, .. }
            | SyncEnd { ts, .. }
            | ThreadStart { ts, .. }
            | ThreadEnd { ts, .. }
            | Wakeup { ts, .. } => ts,
        }
    }

    pub fn id(&self) -> Option<SpanId> {
        use self::TraceEvent::*;
        match *self {
            AsyncStart { id, .. }
            | AsyncOnCPU { id, .. }
            | AsyncOffCPU { id, .. }
            | AsyncEnd { id, .. }
            | SyncStart { id, .. }
            | SyncEnd { id, .. }
            | ThreadStart { id, .. }
            | ThreadEnd { id, .. } => Some(id),
            Wakeup { .. } => None,
        }
    }

    pub fn parent_id(&self) -> Option<SpanId> {
        use self::TraceEvent::*;
        match *self {
            AsyncStart { parent_id, .. } | SyncStart { parent_id, .. } => Some(parent_id),
            SyncEnd { .. }
            | ThreadStart { .. }
            | ThreadEnd { .. }
            | AsyncOnCPU { .. }
            | AsyncOffCPU { .. }
            | AsyncEnd { .. }
            | Wakeup { .. } => None,
        }
    }
}
