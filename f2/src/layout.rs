use std::collections::{BTreeSet,BTreeMap};
use std::time::Duration;
use smallvec::SmallVec;

use event;
use spans;

pub struct LaidSpan<'a> {
    pub span: spans::Span<'a>,
    pub row: u16,
}

pub struct Layout<'a> {
    pub spans: Vec<LaidSpan<'a>>,
    pub total_rows: u16,
}

struct Sweep {
    next_free: u16,
    free: BTreeSet<u16>,
    ends: BTreeSet<(Duration, u16)>,
}

impl Sweep {
    fn new() -> Self {
        Sweep {
            next_free: 0,
            free: BTreeSet::new(),
            ends: BTreeSet::new(),
        }
    }

    fn alloc(&mut self, sp: &spans::Span) -> u16 {
        while let Some((ts, row)) = self.ends.iter().next().cloned() {
            if ts >= sp.start {
                break;
            }
            self.ends.remove(&(ts, row));
            self.free.insert(row);
        }
        let row = if let Some(row) = self.free.iter().next().cloned() {
            self.free.remove(&row);
            row
        } else {
            let row = self.next_free;
            self.next_free += 1;
            row
        };
        self.ends.insert((sp.end, row));
        row
    }
}

pub fn lay_out<'a>(spans: impl Iterator<Item = spans::Span<'a>>) -> Layout<'a> {
    let mut spans: Vec<_> = spans
        .map(|s| LaidSpan {
            span: s,
            row: 0,
        })
        .collect();
    spans.sort_unstable_by_key(|s| s.span.start);

    type Path = SmallVec<[u16; 8]>; // quite possibly the most unnecessary thing
    let mut allocations: BTreeMap<event::SpanId, Path> = BTreeMap::new();
    let mut sweeps: BTreeMap<Path, Sweep> = BTreeMap::new();
    for sp in &mut spans {
        let mut parent_path = if let Some(parent) = sp.span.parent_id {
            allocations.get(&parent).cloned().unwrap_or_else(|| {
                console!(error, format!("Unknown parent span {:?} of {:?}", parent, sp.span.id));
                Path::new()
            })
        } else {
            Path::new()
        };
        let sweep = sweeps.entry(parent_path.clone()).or_insert_with(Sweep::new);
        parent_path.push(sweep.alloc(&sp.span));
        allocations.insert(sp.span.id, parent_path);
    }
    // this could be optimized
    let mut rows: BTreeMap<Path, u16> = allocations.values().cloned().map(|path| (path, 0)).collect();
    for (i, j) in rows.values_mut().enumerate() {
        *j = i as u16;
    }
    for sp in &mut spans {
        sp.row = rows[&allocations[&sp.span.id]];
    }
    Layout {
        spans,
        total_rows: rows.len() as u16,
    }
}
