#![allow(dead_code)]
//! Bounded LIFO calculation history (newest first, max 100 items).
use chrono::Local;

#[derive(Debug, Clone)]
pub struct HistoryRecord {
    pub expr:   String,
    pub result: String,
    pub ts:     String,
}

impl HistoryRecord {
    fn new(expr: impl Into<String>, result: impl Into<String>) -> Self {
        Self {
            expr:   expr.into(),
            result: result.into(),
            ts:     Local::now().format("%H:%M:%S").to_string(),
        }
    }
}

pub struct History {
    records: Vec<HistoryRecord>,
    max:     usize,
    /// Bumped on every mutation so the UI layer can rebuild its model only
    /// when the data actually changed (not on every keypress).
    version: u64,
}

impl History {
    pub fn new() -> Self { Self { records: Vec::new(), max: 100, version: 0 } }

    pub fn push(&mut self, expr: impl Into<String>, result: impl Into<String>) {
        self.records.insert(0, HistoryRecord::new(expr, result));
        self.records.truncate(self.max);
        self.version = self.version.wrapping_add(1);
    }

    pub fn all(&self)  -> &[HistoryRecord] { &self.records }

    pub fn clear(&mut self) {
        if !self.records.is_empty() {
            self.records.clear();
            self.version = self.version.wrapping_add(1);
        }
    }

    pub fn len(&self)  -> usize            { self.records.len() }
    pub fn is_empty(&self) -> bool         { self.records.is_empty() }
    pub fn version(&self) -> u64           { self.version }

    pub fn get(&self, idx: usize) -> Option<&HistoryRecord> { self.records.get(idx) }
}

impl Default for History { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_and_bounded() {
        let mut h = History::new();
        for i in 0..105 {
            h.push(format!("expr{i}"), format!("res{i}"));
        }
        assert_eq!(h.len(), 100);
        assert_eq!(h.get(0).unwrap().expr, "expr104"); // newest first
        assert_eq!(h.get(99).unwrap().expr, "expr5");  // oldest survivor
    }

    #[test]
    fn version_tracks_mutations() {
        let mut h = History::new();
        let v0 = h.version();
        h.push("a", "1");
        assert_eq!(h.version(), v0 + 1);
        h.clear();
        assert_eq!(h.version(), v0 + 2);
        h.clear(); // no-op clear must not bump
        assert_eq!(h.version(), v0 + 2);
    }
}
