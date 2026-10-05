//! Bounded ephemeral undo/redo over complete canonical authoring sessions.
//!
//! History is deliberately not nested into `ALGS` and is not an interchange
//! format. Every retained snapshot is one complete canonical `ALGS` byte
//! string, so navigation replays the same exact boundary as persistence and
//! file import before changing history state.

use core::fmt;

use super::{
    CanonicalGraphAuthoringSessionEncoding, GraphAuthoringSessionError,
    GraphAuthoringSessionReplay, GraphAuthoringSessionReplayLimits, replay_graph_authoring_session,
};

/// Bounded retention policy for complete canonical `ALGS` snapshots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphAuthoringSessionHistoryLimits {
    /// Maximum retained snapshots on each side of the current session.
    pub maximum_snapshots_per_direction: usize,
    /// Maximum combined canonical bytes retained by both stacks.
    pub maximum_history_bytes: usize,
}

impl GraphAuthoringSessionHistoryLimits {
    /// First browser/native authoring-history policy. The current session is
    /// held by the editor and does not count against this byte budget.
    pub const fn interactive() -> Self {
        Self {
            maximum_snapshots_per_direction: 16,
            maximum_history_bytes: 64 * 1024 * 1024,
        }
    }

    fn validate(self) -> Result<(), GraphAuthoringSessionHistoryError> {
        if self.maximum_snapshots_per_direction == 0 || self.maximum_history_bytes == 0 {
            Err(GraphAuthoringSessionHistoryError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphAuthoringSessionHistoryLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GraphAuthoringSessionSnapshot {
    bytes: Vec<u8>,
}

impl GraphAuthoringSessionSnapshot {
    fn from_encoding(encoding: CanonicalGraphAuthoringSessionEncoding) -> Self {
        Self {
            bytes: encoding.into_bytes(),
        }
    }

    fn byte_length(&self) -> usize {
        self.bytes.len()
    }
}

/// Bounded ephemeral history around one complete exact authoring session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphAuthoringSessionHistory {
    limits: GraphAuthoringSessionHistoryLimits,
    undo: Vec<GraphAuthoringSessionSnapshot>,
    redo: Vec<GraphAuthoringSessionSnapshot>,
    retained_bytes: usize,
}

impl GraphAuthoringSessionHistory {
    /// Construct empty history under an explicit retention policy.
    pub fn try_new(
        limits: GraphAuthoringSessionHistoryLimits,
    ) -> Result<Self, GraphAuthoringSessionHistoryError> {
        limits.validate()?;
        Ok(Self {
            limits,
            undo: Vec::new(),
            redo: Vec::new(),
            retained_bytes: 0,
        })
    }

    /// Return the retained policy.
    pub const fn limits(&self) -> GraphAuthoringSessionHistoryLimits {
        self.limits
    }

    /// Return available undo transitions.
    pub const fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Return available redo transitions.
    pub const fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Return combined canonical `ALGS` bytes retained by both stacks.
    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    /// Return whether a prior complete session exists.
    pub const fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Return whether a later complete session exists.
    pub const fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Replay the next undo target without changing navigation state.
    pub fn preview_undo(
        &self,
        admission: GraphAuthoringSessionReplayLimits,
    ) -> Result<Option<GraphAuthoringSessionReplay>, GraphAuthoringSessionHistoryError> {
        self.undo
            .last()
            .map(|target| replay_snapshot(target, admission))
            .transpose()
    }

    /// Replay the next redo target without changing navigation state.
    pub fn preview_redo(
        &self,
        admission: GraphAuthoringSessionReplayLimits,
    ) -> Result<Option<GraphAuthoringSessionReplay>, GraphAuthoringSessionHistoryError> {
        self.redo
            .last()
            .map(|target| replay_snapshot(target, admission))
            .transpose()
    }

    /// Record a successfully replaced prior complete session and clear the
    /// abandoned redo branch. Oldest snapshots are evicted until both bounds
    /// hold.
    pub fn record(
        &mut self,
        prior: CanonicalGraphAuthoringSessionEncoding,
    ) -> Result<(), GraphAuthoringSessionHistoryError> {
        let prior = GraphAuthoringSessionSnapshot::from_encoding(prior);
        self.require_snapshot_size(&prior)?;
        let redo_bytes = snapshot_bytes(&self.redo)?;
        let retained_without_redo = self
            .retained_bytes
            .checked_sub(redo_bytes)
            .ok_or(GraphAuthoringSessionHistoryError::ByteAccounting)?;
        let retained_with_prior = retained_without_redo
            .checked_add(prior.byte_length())
            .ok_or(GraphAuthoringSessionHistoryError::ByteAccounting)?;
        self.redo.clear();
        self.retained_bytes = retained_with_prior;
        self.undo.push(prior);
        self.enforce_limits();
        Ok(())
    }

    /// Navigate to the newest prior complete session after exact replay.
    /// Failure leaves navigation state and the caller's current session intact.
    pub fn undo(
        &mut self,
        current: CanonicalGraphAuthoringSessionEncoding,
        admission: GraphAuthoringSessionReplayLimits,
    ) -> Result<Option<GraphAuthoringSessionReplay>, GraphAuthoringSessionHistoryError> {
        self.navigate(current, admission, false)
    }

    /// Navigate to the newest later complete session after exact replay.
    /// Failure leaves navigation state and the caller's current session intact.
    pub fn redo(
        &mut self,
        current: CanonicalGraphAuthoringSessionEncoding,
        admission: GraphAuthoringSessionReplayLimits,
    ) -> Result<Option<GraphAuthoringSessionReplay>, GraphAuthoringSessionHistoryError> {
        self.navigate(current, admission, true)
    }

    /// Clear all ephemeral navigation state without changing the current
    /// authoring session.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.retained_bytes = 0;
    }

    fn navigate(
        &mut self,
        current: CanonicalGraphAuthoringSessionEncoding,
        admission: GraphAuthoringSessionReplayLimits,
        redo: bool,
    ) -> Result<Option<GraphAuthoringSessionReplay>, GraphAuthoringSessionHistoryError> {
        let target = if redo {
            self.redo.last()
        } else {
            self.undo.last()
        };
        let Some(target) = target else {
            return Ok(None);
        };
        let replay = replay_snapshot(target, admission)?;
        let target_byte_length = target.byte_length();
        let current = GraphAuthoringSessionSnapshot::from_encoding(current);
        self.require_snapshot_size(&current)?;
        let retained_without_target = self
            .retained_bytes
            .checked_sub(target_byte_length)
            .ok_or(GraphAuthoringSessionHistoryError::ByteAccounting)?;
        let retained_with_current = retained_without_target
            .checked_add(current.byte_length())
            .ok_or(GraphAuthoringSessionHistoryError::ByteAccounting)?;
        if redo {
            self.redo.pop().expect("checked nonempty redo stack");
            self.undo.push(current);
        } else {
            self.undo.pop().expect("checked nonempty undo stack");
            self.redo.push(current);
        }
        self.retained_bytes = retained_with_current;
        self.enforce_limits();
        Ok(Some(replay))
    }

    fn require_snapshot_size(
        &self,
        snapshot: &GraphAuthoringSessionSnapshot,
    ) -> Result<(), GraphAuthoringSessionHistoryError> {
        if snapshot.byte_length() > self.limits.maximum_history_bytes {
            Err(GraphAuthoringSessionHistoryError::SnapshotTooLarge)
        } else {
            Ok(())
        }
    }

    fn enforce_limits(&mut self) {
        while self.undo.len() > self.limits.maximum_snapshots_per_direction {
            let removed = self.undo.remove(0);
            self.retained_bytes -= removed.byte_length();
        }
        while self.redo.len() > self.limits.maximum_snapshots_per_direction {
            let removed = self.redo.remove(0);
            self.retained_bytes -= removed.byte_length();
        }
        while self.retained_bytes > self.limits.maximum_history_bytes {
            let removed = if !self.undo.is_empty() {
                self.undo.remove(0)
            } else if !self.redo.is_empty() {
                self.redo.remove(0)
            } else {
                break;
            };
            self.retained_bytes -= removed.byte_length();
        }
    }
}

impl Default for GraphAuthoringSessionHistory {
    fn default() -> Self {
        Self::try_new(GraphAuthoringSessionHistoryLimits::interactive())
            .expect("interactive authoring-session history limits are nonzero")
    }
}

fn replay_snapshot(
    snapshot: &GraphAuthoringSessionSnapshot,
    admission: GraphAuthoringSessionReplayLimits,
) -> Result<GraphAuthoringSessionReplay, GraphAuthoringSessionHistoryError> {
    replay_graph_authoring_session(&snapshot.bytes, admission).map_err(Into::into)
}

fn snapshot_bytes(
    snapshots: &[GraphAuthoringSessionSnapshot],
) -> Result<usize, GraphAuthoringSessionHistoryError> {
    snapshots.iter().try_fold(0_usize, |total, snapshot| {
        total
            .checked_add(snapshot.byte_length())
            .ok_or(GraphAuthoringSessionHistoryError::ByteAccounting)
    })
}

/// Rejection at the bounded exact authoring-session history boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphAuthoringSessionHistoryError {
    /// A retention policy contained zero.
    ZeroLimit,
    /// One complete canonical `ALGS` snapshot exceeded the history byte budget.
    SnapshotTooLarge,
    /// Checked combined-byte accounting failed.
    ByteAccounting,
    /// Complete canonical authoring-session replay rejected a snapshot.
    Session(GraphAuthoringSessionError),
}

impl From<GraphAuthoringSessionError> for GraphAuthoringSessionHistoryError {
    fn from(value: GraphAuthoringSessionError) -> Self {
        Self::Session(value)
    }
}

impl fmt::Display for GraphAuthoringSessionHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => {
                formatter.write_str("authoring-session history policy contains zero")
            }
            Self::SnapshotTooLarge => {
                formatter.write_str("canonical authoring-session snapshot exceeds history policy")
            }
            Self::ByteAccounting => {
                formatter.write_str("authoring-session history byte accounting failed")
            }
            Self::Session(error) => write!(formatter, "authoring-session history replay: {error}"),
        }
    }
}

impl std::error::Error for GraphAuthoringSessionHistoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        GraphAuthoringSessionDocument, GraphAuthoringSessionLimits, GraphHierarchySourceMapLimits,
        GraphNodeId, GraphNodePlacement, GraphProbeDocument, GraphProbeLimits,
        GraphWorkspaceDocument, GraphWorkspaceLimits, compile_representative_exact_control_graph,
        encode_graph_authoring_session,
    };

    fn session(
        control_offset: i32,
        probe_revision: u64,
        cached_job_offset: i32,
    ) -> (
        GraphAuthoringSessionDocument,
        CanonicalGraphAuthoringSessionEncoding,
    ) {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let graph = fixture.document().clone();
        let placements = graph
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| {
                GraphNodePlacement::new(
                    node.id(),
                    i32::try_from(index).unwrap() * 10,
                    i32::try_from(index % 3).unwrap() * 10,
                )
            })
            .collect();
        let next_node = graph
            .nodes()
            .iter()
            .map(|node| u64::from(node.id().get()))
            .max()
            .unwrap()
            + 1;
        let next_wire = graph
            .wires()
            .iter()
            .map(|wire| u64::from(wire.id().get()))
            .max()
            .unwrap()
            + 1;
        let base = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            next_node,
            next_wire,
            graph,
            placements,
        )
        .unwrap();
        let mut control = base.clone();
        if control_offset != 0 {
            let id = GraphNodeId::new(1);
            let placement = control.placement(id).unwrap();
            control
                .move_node(id, placement.x() + control_offset, placement.y())
                .unwrap();
        }
        let probes = GraphProbeDocument::try_new(
            GraphProbeLimits::interactive(),
            probe_revision,
            1,
            None,
            &control,
            Vec::new(),
        )
        .unwrap();
        let mut cached_job = base;
        if cached_job_offset != 0 {
            let id = GraphNodeId::new(2);
            let placement = cached_job.placement(id).unwrap();
            cached_job
                .move_node(id, placement.x(), placement.y() + cached_job_offset)
                .unwrap();
        }
        let document = GraphAuthoringSessionDocument::try_new(
            GraphAuthoringSessionLimits::interactive(),
            GraphHierarchySourceMapLimits::interactive(),
            control,
            probes,
            cached_job,
            None,
        )
        .unwrap();
        let encoding = encode_graph_authoring_session(&document).unwrap();
        (document, encoding)
    }

    #[test]
    fn mixed_undo_redo_replays_every_complete_session_section() {
        let (initial, initial_encoding) = session(0, 1, 0);
        let (graph_edit, graph_encoding) = session(7, 1, 0);
        let (probe_edit, probe_encoding) = session(7, 2, 0);
        let (_, cached_job_encoding) = session(7, 2, 9);
        let mut history = GraphAuthoringSessionHistory::default();
        history.record(initial_encoding).unwrap();
        history.record(graph_encoding).unwrap();
        history.record(probe_encoding).unwrap();
        assert_eq!((history.undo_len(), history.redo_len()), (3, 0));

        let probe_target = history
            .undo(
                cached_job_encoding,
                GraphAuthoringSessionReplayLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(probe_target.document(), &probe_edit);
        let graph_target = history
            .undo(
                probe_target.encoding().clone(),
                GraphAuthoringSessionReplayLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(graph_target.document(), &graph_edit);
        let initial_target = history
            .undo(
                graph_target.encoding().clone(),
                GraphAuthoringSessionReplayLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(initial_target.document(), &initial);
        assert_eq!((history.undo_len(), history.redo_len()), (0, 3));

        let redone = history
            .redo(
                initial_target.encoding().clone(),
                GraphAuthoringSessionReplayLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(redone.document(), &graph_edit);
        assert_eq!((history.undo_len(), history.redo_len()), (1, 2));
    }

    #[test]
    fn abandoned_redo_and_oldest_snapshot_eviction_are_complete() {
        let (_, initial) = session(0, 1, 0);
        let (graph_document, graph_edit) = session(3, 1, 0);
        let (_, probe_edit) = session(3, 2, 0);
        let (_, cached_job_edit) = session(3, 2, 4);
        let mut history =
            GraphAuthoringSessionHistory::try_new(GraphAuthoringSessionHistoryLimits {
                maximum_snapshots_per_direction: 2,
                maximum_history_bytes: 64 * 1024 * 1024,
            })
            .unwrap();
        history.record(initial).unwrap();
        history.record(graph_edit.clone()).unwrap();
        history.record(probe_edit).unwrap();
        assert_eq!(history.undo_len(), 2);
        let probe_target = history
            .undo(
                cached_job_edit,
                GraphAuthoringSessionReplayLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        let graph_target = history
            .undo(
                probe_target.encoding().clone(),
                GraphAuthoringSessionReplayLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(graph_target.document(), &graph_document);
        assert!(
            !history.can_undo(),
            "the evicted initial session resurfaced"
        );
        let probe_target = history
            .redo(
                graph_target.encoding().clone(),
                GraphAuthoringSessionReplayLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(history.redo_len(), 1);
        history.record(probe_target.encoding().clone()).unwrap();
        assert_eq!(history.redo_len(), 0);
    }

    #[test]
    fn limits_and_corrupt_targets_fail_without_history_mutation() {
        assert_eq!(
            GraphAuthoringSessionHistory::try_new(GraphAuthoringSessionHistoryLimits {
                maximum_snapshots_per_direction: 0,
                maximum_history_bytes: 1,
            }),
            Err(GraphAuthoringSessionHistoryError::ZeroLimit)
        );
        let (_, initial) = session(0, 1, 0);
        let (_, current) = session(5, 1, 0);
        let mut too_small =
            GraphAuthoringSessionHistory::try_new(GraphAuthoringSessionHistoryLimits {
                maximum_snapshots_per_direction: 1,
                maximum_history_bytes: initial.bytes().len() - 1,
            })
            .unwrap();
        assert_eq!(
            too_small.record(initial.clone()),
            Err(GraphAuthoringSessionHistoryError::SnapshotTooLarge)
        );

        let mut history = GraphAuthoringSessionHistory::default();
        history.record(initial).unwrap();
        history.undo.last_mut().unwrap().bytes[0] ^= 0xff;
        let retained = history.clone();
        assert_eq!(
            history.undo(current, GraphAuthoringSessionReplayLimits::interactive(),),
            Err(GraphAuthoringSessionHistoryError::Session(
                GraphAuthoringSessionError::InvalidMagic
            ))
        );
        assert_eq!(history, retained);
    }

    #[test]
    fn tighter_nested_replay_policy_fails_transactionally() {
        let (_, initial) = session(0, 1, 0);
        let (_, current) = session(6, 1, 0);
        let mut history = GraphAuthoringSessionHistory::default();
        history.record(initial.clone()).unwrap();
        let retained = history.clone();
        let mut tight = GraphAuthoringSessionReplayLimits::interactive();
        tight.session.maximum_session_bytes = initial.bytes().len() - 1;
        assert!(matches!(
            history.undo(current, tight),
            Err(GraphAuthoringSessionHistoryError::Session(
                GraphAuthoringSessionError::LimitExceeded("admitted document byte length")
            ))
        ));
        assert_eq!(history, retained);
    }
}
