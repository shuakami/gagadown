//! Segment table with IDM-style dynamic splitting: an idle worker takes the upper half of
//! whichever segment has the most bytes left, so every connection stays busy until the end.
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Seg {
    pub start: u64,
    /// Bytes reserved by the owning worker (received from the network).
    pub pos: u64,
    /// Bytes durably written to the part file.
    pub done: u64,
    pub end: u64,
    pub worker: Option<u32>,
    pub claimed_at: Option<Instant>,
    pub window: u64,
    pub speed: f64,
    /// Set by the controller to make a straggler drop its connection and re-claim.
    pub abort: bool,
}

impl Seg {
    fn new(start: u64, done: u64, end: u64) -> Self {
        Self { start, pos: done, done, end, worker: None, claimed_at: None, window: 0, speed: 0.0, abort: false }
    }
    pub fn left(&self) -> u64 {
        self.end.saturating_sub(self.pos)
    }
    pub fn finished(&self) -> bool {
        self.done >= self.end
    }
}

pub struct Reserve {
    pub take: usize,
    pub complete: bool,
    pub abort: bool,
}

#[derive(Debug)]
pub struct Table {
    pub size: u64,
    pub segs: Vec<Seg>,
    pub splits: u64,
}

const ALIGN: u64 = 64 * 1024;

impl Table {
    pub fn fresh(size: u64, parts: usize, min_split: u64) -> Self {
        let parts = (parts as u64).clamp(1, (size / min_split.max(1)).max(1));
        let mut segs = Vec::with_capacity(parts as usize);
        let step = size / parts;
        let mut start = 0;
        for i in 0..parts {
            let end = if i + 1 == parts { size } else { ((start + step) / ALIGN * ALIGN).max(start + 1).min(size) };
            segs.push(Seg::new(start, start, end));
            start = end;
        }
        Self { size, segs, splits: 0 }
    }

    /// Rebuild from persisted holes; covered gaps become finished segments.
    pub fn from_remaining(size: u64, remaining: &[(u64, u64)]) -> Self {
        let mut holes: Vec<(u64, u64)> = remaining.iter().copied().filter(|(a, b)| a < b && *b <= size).collect();
        holes.sort();
        let mut segs = Vec::new();
        let mut cur = 0;
        for (a, b) in holes {
            let a = a.max(cur);
            if a >= b {
                continue;
            }
            if a > cur {
                segs.push(Seg::new(cur, a, a));
            }
            segs.push(Seg::new(a, a, b));
            cur = b;
        }
        if cur < size {
            segs.push(Seg::new(cur, size, size));
        }
        Self { size, segs, splits: 0 }
    }

    pub fn remaining(&self) -> Vec<(u64, u64)> {
        let mut v: Vec<(u64, u64)> = self.segs.iter().filter(|s| !s.finished()).map(|s| (s.done, s.end)).collect();
        v.sort();
        v
    }

    pub fn downloaded(&self) -> u64 {
        self.segs.iter().map(|s| s.done - s.start).sum()
    }

    pub fn left_total(&self) -> u64 {
        self.segs.iter().map(|s| s.end - s.done).sum()
    }

    pub fn complete(&self) -> bool {
        self.segs.iter().all(|s| s.finished())
    }

    pub fn active(&self) -> usize {
        self.segs.iter().filter(|s| s.worker.is_some()).count()
    }

    pub fn has_work(&self, min_split: u64) -> bool {
        self.segs.iter().any(|s| s.worker.is_none() && !s.finished())
            || self.segs.iter().any(|s| s.worker.is_some() && s.left() >= 2 * min_split)
    }

    pub fn claim(&mut self, worker: u32, min_split: u64) -> Option<usize> {
        let now = Instant::now();
        let idle = self
            .segs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.worker.is_none() && !s.finished())
            .max_by_key(|(_, s)| s.end - s.done)
            .map(|(i, _)| i);
        if let Some(i) = idle {
            let s = &mut self.segs[i];
            s.worker = Some(worker);
            s.pos = s.done;
            s.claimed_at = Some(now);
            s.abort = false;
            s.window = 0;
            s.speed = 0.0;
            return Some(i);
        }
        // Steal: split the busy segment with the largest tail. Slow segments get split
        // first when sizes tie because they will take the longest to drain.
        let victim = self
            .segs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.worker.is_some() && s.left() >= 2 * min_split)
            .max_by(|(_, a), (_, b)| {
                let ta = a.left() as f64 / a.speed.max(1.0);
                let tb = b.left() as f64 / b.speed.max(1.0);
                ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)?;
        let v = &mut self.segs[victim];
        let mut mid = v.pos + v.left() / 2;
        let aligned = mid / ALIGN * ALIGN;
        if aligned > v.pos + min_split / 2 {
            mid = aligned;
        }
        let end = v.end;
        v.end = mid;
        let mut ns = Seg::new(mid, mid, end);
        ns.worker = Some(worker);
        ns.claimed_at = Some(now);
        self.segs.push(ns);
        self.splits += 1;
        Some(self.segs.len() - 1)
    }

    pub fn reserve(&mut self, idx: usize, len: usize) -> Reserve {
        let s = &mut self.segs[idx];
        let take = (len as u64).min(s.left()) as usize;
        s.pos += take as u64;
        s.window += take as u64;
        Reserve { take, complete: s.pos >= s.end, abort: s.abort }
    }

    pub fn commit(&mut self, idx: usize, n: u64) {
        let s = &mut self.segs[idx];
        s.done = (s.done + n).min(s.end);
    }

    pub fn release(&mut self, idx: usize) {
        let s = &mut self.segs[idx];
        s.worker = None;
        s.pos = s.done;
        s.claimed_at = None;
        s.abort = false;
        s.speed = 0.0;
    }

    /// Down-sampled view for drawing: (start, done, end, active).
    pub fn view(&self, max: usize) -> Vec<(u64, u64, u64, bool)> {
        let mut v: Vec<(u64, u64, u64, bool)> = self.segs.iter().map(|s| (s.start, s.done, s.end, s.worker.is_some())).collect();
        v.sort_by_key(|x| x.0);
        if v.len() <= max {
            return v;
        }
        // Merge neighbours that are both finished first.
        let mut out: Vec<(u64, u64, u64, bool)> = Vec::with_capacity(v.len());
        for s in v {
            if let Some(last) = out.last_mut() {
                if last.1 == last.2 && s.1 == s.2 && !last.3 && !s.3 {
                    last.2 = s.2;
                    last.1 = s.2;
                    continue;
                }
            }
            out.push(s);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_partition() {
        let mut t = Table::fresh(10 * 1024 * 1024, 2, 256 * 1024);
        let a = t.claim(0, 256 * 1024).unwrap();
        let b = t.claim(1, 256 * 1024).unwrap();
        assert_ne!(a, b);
        let c = t.claim(2, 256 * 1024).unwrap();
        assert_eq!(t.segs.len(), 3);
        let mut ranges: Vec<_> = t.segs.iter().map(|s| (s.start, s.end)).collect();
        ranges.sort();
        assert_eq!(ranges[0].0, 0);
        for w in ranges.windows(2) {
            assert_eq!(w[0].1, w[1].0);
        }
        assert_eq!(ranges.last().unwrap().1, t.size);
        let r = t.reserve(c, 1 << 30);
        assert!(r.complete);
    }

    #[test]
    fn remaining_roundtrip() {
        let t = Table::from_remaining(1000, &[(100, 200), (500, 1000)]);
        assert_eq!(t.remaining(), vec![(100, 200), (500, 1000)]);
        assert_eq!(t.downloaded(), 100 + 300);
    }
}
