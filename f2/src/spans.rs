use std::collections::HashMap;
use std::time::Duration;
use std::borrow::Cow;

use event::*;

#[derive(Debug)]
pub struct State {
    pub active_spans: HashMap<SpanId, ActiveSpan>,

    pub finished_spans: Vec<Span<'static>>,

    pub end_time: Duration,
}

#[derive(Debug)]
pub struct ActiveSpan {
    pub event: TraceEvent,
    pub message: Vec<u8>,
    pub wakeups: Vec<Wakeup>,
}

impl ActiveSpan {
    fn in_progress<'a>(&'a self, ts: Duration) -> Span<'a> {
        Span {
            id: self.event.id().unwrap(),
            parent_id: self.event.parent_id(),
            start: self.event.ts(),
            end: ts,
            style: match self.event {
                TraceEvent::AsyncStart { .. } => SpanStyle::AsyncInProgress,
                TraceEvent::SyncStart { .. } => SpanStyle::SyncInProgress,
                TraceEvent::ThreadStart { .. } => SpanStyle::ThreadInProgress,
                _ => panic!("wrong kind of start event"),
            },
            message: Cow::Borrowed(&self.message),
            wakeups: Cow::Borrowed(&self.wakeups),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Wakeup {
    target: SpanId,
    ts: Duration,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SpanStyle {
    ThreadInProgress,
    ThreadFinished,
    SyncInProgress,
    SyncFinished,
    AsyncInProgress,
    AsyncSuccess,
    AsyncCancel,
    AsyncError,
}

#[derive(Debug)]
pub struct Span<'a> {
    pub id: SpanId,
    pub parent_id: Option<SpanId>,
    pub start: Duration,
    pub end: Duration,
    pub message: Cow<'a, [u8]>,
    pub wakeups: Cow<'a, [Wakeup]>,

    pub style: SpanStyle,
    // TODO: more complicated stuff goes here
}

impl<'a> Span<'a> {
    pub fn borrow<'b>(&'b self) -> Span<'b> {
        Span {
            id: self.id,
            parent_id: self.parent_id,
            start: self.start,
            end: self.end,
            message: Cow::from(&self.message[..]),
            wakeups: Cow::from(&self.wakeups[..]),
            style: self.style,
        }
    }
}

impl State {
    pub fn new() -> Self {
        State {
            active_spans: HashMap::new(),
            finished_spans: Vec::new(),
            end_time: Duration::default(),
        }
    }

    pub fn bump_time(&mut self, ts: Duration) {
        if ts > self.end_time {
            self.end_time = ts;
        }
    }

    pub fn add_event(&mut self, event: TraceEvent) {
        self.bump_time(event.ts());
        match event {
            TraceEvent::AsyncStart { id, .. }
            | TraceEvent::SyncStart { id, .. }
            | TraceEvent::ThreadStart { id, .. } => {
                self.active_spans.insert(
                    id,
                    ActiveSpan {
                        wakeups: vec![],
                        message: match event {
                            TraceEvent::AsyncStart {
                                ref name,
                                ref metadata,
                                ..
                            } => format!("{} {}", name, metadata),
                            TraceEvent::SyncStart {
                                ref name,
                                ref metadata,
                                ..
                            } => format!("{} {}", name, metadata),
                            TraceEvent::ThreadStart { ref name, .. } => name.clone(),
                            _ => unreachable!(),
                        }.into_bytes(),
                        event,
                    },
                );
            }
            TraceEvent::AsyncOnCPU { .. } | TraceEvent::AsyncOffCPU { .. } => {
                // TODO
            }
            TraceEvent::AsyncEnd { id, ts, .. }
            | TraceEvent::SyncEnd { id, ts }
            | TraceEvent::ThreadEnd { id, ts } => {
                if let Some(mut start) = self.active_spans.remove(&id) {
                    self.finished_spans.push(Span {
                        id,
                        parent_id: start.event.parent_id(),
                        start: start.event.ts(),
                        end: ts,
                        style: match start.event {
                            TraceEvent::AsyncStart { .. } => match event {
                                TraceEvent::AsyncEnd {
                                    outcome: AsyncOutcome::Success,
                                    ..
                                } => SpanStyle::AsyncSuccess,
                                TraceEvent::AsyncEnd {
                                    outcome: AsyncOutcome::Cancelled,
                                    ..
                                } => SpanStyle::AsyncCancel,
                                TraceEvent::AsyncEnd {
                                    outcome: AsyncOutcome::Error(_),
                                    ..
                                } => SpanStyle::AsyncError,
                                _ => {
                                    eprintln!("wrong kind of start event");
                                    return;
                                }
                            },
                            TraceEvent::SyncStart { .. } => SpanStyle::SyncFinished,
                            TraceEvent::ThreadStart { .. } => SpanStyle::ThreadFinished,
                            _ => {
                                eprintln!("wrong kind of start event");
                                return;
                            }
                        },
                        message: start.message.into(),
                        wakeups: start.wakeups.into(),
                    });
                } else {
                    eprintln!("unknown span id: {:?}", id);
                }
            }
            TraceEvent::Wakeup {
                waking_span,
                parked_span,
                ts,
            } => {
                if let Some(sp) = self.active_spans.get_mut(&waking_span) {
                    sp.wakeups.push(Wakeup {
                        target: parked_span,
                        ts,
                    });
                } else {
                    eprintln!("unknown waking span id: {:?}", waking_span);
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.active_spans.len() + self.finished_spans.len()
    }

    pub fn select<'a>(
        &'a self,
        start: Duration,
        end: Duration,
    ) -> impl Iterator<Item = Span<'a>> + 'a {
        // FIXME: make this good and not bad
        self.active_spans.values().filter(move |e| e.event.ts() < end)
            .map(move |e| e.in_progress(end)) // FIXME
            .chain(self.finished_spans.iter()
                   .filter(move |s| s.start < end && s.end > start)
                   .map(|s| s.borrow()))
    }
}
