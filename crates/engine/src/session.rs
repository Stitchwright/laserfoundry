use laserfoundry_doc::{Document, ShapeId};
use serde_json::Value;

use crate::{EngineError, cmd, guard};

/// Undo steps kept.
pub const HISTORY_LIMIT: usize = 200;
/// Roughly how much memory the undo steps may use together (the newest step is always kept).
pub const HISTORY_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Snapshot {
    doc: Document,
    selection: Vec<ShapeId>,
    /// Estimated memory of `doc` (see [`Document::approx_bytes`]).
    bytes: usize,
}

/// One open document with its selection and history.
pub struct Session {
    pub doc: Document,
    pub selection: Vec<ShapeId>,
    /// The most recently generated program (`job.gcode`), kept so the UI can show or save it.
    pub last_gcode: Option<String>,
    pub path: Option<std::path::PathBuf>,
    pub dirty: bool,
    /// Bumped on every change to the document; lets the UI cache derived data (previews).
    pub revision: u64,
    /// Output only the selected shapes (generate, estimate, frame and preview all follow it).
    pub job_selection_only: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The undo memory budget (a field so tests can make it small).
    history_bytes: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self::with_document(Document::default())
    }

    pub fn with_document(doc: Document) -> Self {
        Session {
            doc,
            selection: Vec::new(),
            last_gcode: None,
            path: None,
            dirty: false,
            revision: 0,
            job_selection_only: false,
            undo: Vec::new(),
            redo: Vec::new(),
            history_bytes: HISTORY_BYTES,
        }
    }

    /// The document as the job sees it: all of it, or only the selected shapes when the person has
    /// chosen to cut just the selection.
    pub fn job_doc(&self) -> std::borrow::Cow<'_, Document> {
        if !self.job_selection_only {
            return std::borrow::Cow::Borrowed(&self.doc);
        }
        let mut doc = self.doc.clone();
        doc.shapes.retain(|s| self.selection.contains(&s.id));
        std::borrow::Cow::Owned(doc)
    }

    /// Runs a command by id. Mutating commands are one undo step; a failure or panic leaves the
    /// document exactly as it was.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, EngineError> {
        let spec = cmd::find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        self.execute_spec(spec, params)
    }

    fn execute_spec(&mut self, spec: &cmd::CommandSpec, params: &Value) -> Result<Value, EngineError> {
        if !spec.mutates {
            return guard::guarded(|| (spec.run)(self, params));
        }
        let before = self.snapshot();
        match guard::guarded(|| (spec.run)(self, params)) {
            Ok(v) => {
                self.push_undo(before);
                self.dirty = true;
                self.revision += 1;
                Ok(v)
            }
            Err(e) => {
                self.restore(before);
                Err(e)
            }
        }
    }

    /// Replaces the document (new/open). History does not carry across documents.
    pub(crate) fn replace_document(&mut self, doc: Document, path: Option<std::path::PathBuf>) {
        self.doc = doc;
        self.selection.clear();
        self.last_gcode = None;
        self.path = path;
        self.dirty = false;
        self.revision += 1;
        self.undo.clear();
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        let current = self.snapshot();
        self.redo.push(current);
        self.restore(prev);
        true
    }

    pub(crate) fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        let current = self.snapshot();
        self.undo.push(current);
        self.restore(next);
        true
    }

    /// Runs a command inside another one (batch) without creating its own undo step.
    pub(crate) fn run_nested(&mut self, id: &str, params: &Value) -> Result<Value, EngineError> {
        let spec = cmd::find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        (spec.run)(self, params)
    }

    fn push_undo(&mut self, snap: Snapshot) {
        self.undo.push(snap);
        let mut total: usize = self.undo.iter().map(|s| s.bytes).sum();
        while self.undo.len() > HISTORY_LIMIT || (total > self.history_bytes && self.undo.len() > 1) {
            total = total.saturating_sub(self.undo.remove(0).bytes);
        }
        self.redo.clear();
    }

    #[cfg(test)]
    pub(crate) fn set_history_bytes_for_test(&mut self, bytes: usize) {
        self.history_bytes = bytes;
    }

    #[cfg(test)]
    pub(crate) fn execute_spec_for_test(&mut self, spec: &cmd::CommandSpec, params: &Value) -> Result<Value, EngineError> {
        self.execute_spec(spec, params)
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot { doc: self.doc.clone(), selection: self.selection.clone(), bytes: self.doc.approx_bytes() }
    }

    pub(crate) fn restore(&mut self, s: Snapshot) {
        self.revision += 1;
        self.doc = s.doc;
        self.selection = s.selection;
    }
}
