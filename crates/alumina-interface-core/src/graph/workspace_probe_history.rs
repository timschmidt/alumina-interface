//! Bounded ephemeral undo/redo over exact workspace/probe pairs.
//!
//! History is deliberately not a canonical interchange format. Each retained
//! entry contains the two independent canonical artifacts and navigation
//! replays both, including the probe sidecar's exact external-workspace
//! binding, before changing history state.

use core::fmt;

use super::{
    CanonicalGraphProbeEncoding, CanonicalGraphWorkspaceEncoding, GraphLimits, GraphProbeDocument,
    GraphProbeError, GraphProbeLimits, GraphProbeReplay, GraphWorkspaceDocument,
    GraphWorkspaceError, GraphWorkspaceLimits, GraphWorkspaceReplay, encode_graph_probes,
    encode_graph_workspace, replay_graph_probes, replay_graph_workspace,
};

/// Bounded retention policy for paired ALGW/ALGP editor snapshots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphWorkspaceProbeHistoryLimits {
    /// Maximum retained paired snapshots on each side of the current pair.
    pub maximum_snapshots_per_direction: usize,
    /// Maximum combined ALGW plus ALGP bytes retained by both stacks.
    pub maximum_history_bytes: usize,
}

impl GraphWorkspaceProbeHistoryLimits {
    /// Interactive pair-history policy. The current pair is held separately
    /// and does not count against this byte budget.
    pub const fn interactive() -> Self {
        Self {
            maximum_snapshots_per_direction: 32,
            maximum_history_bytes: 64 * 1024 * 1024,
        }
    }

    fn validate(self) -> Result<(), GraphWorkspaceProbeHistoryError> {
        if self.maximum_snapshots_per_direction == 0 || self.maximum_history_bytes == 0 {
            Err(GraphWorkspaceProbeHistoryError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphWorkspaceProbeHistoryLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GraphWorkspaceProbeSnapshot {
    workspace: CanonicalGraphWorkspaceEncoding,
    probes: CanonicalGraphProbeEncoding,
    byte_length: usize,
}

impl GraphWorkspaceProbeSnapshot {
    fn capture(
        workspace: &GraphWorkspaceDocument,
        probes: &GraphProbeDocument,
    ) -> Result<Self, GraphWorkspaceProbeHistoryError> {
        let workspace_encoding = encode_graph_workspace(workspace)?;
        let probe_encoding = encode_graph_probes(probes)?;
        // Replaying the freshly encoded sidecar is the shared exact binding
        // check and also rejects a caller pairing it with another workspace.
        replay_graph_probes(probe_encoding.bytes(), workspace, probes.limits())?;
        let byte_length = workspace_encoding
            .bytes()
            .len()
            .checked_add(probe_encoding.bytes().len())
            .ok_or(GraphWorkspaceProbeHistoryError::ByteAccounting)?;
        Ok(Self {
            workspace: workspace_encoding,
            probes: probe_encoding,
            byte_length,
        })
    }
}

/// A paired history target after independent canonical ALGW and bound-ALGP
/// replay.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphWorkspaceProbeHistoryReplay {
    workspace: GraphWorkspaceReplay,
    probes: GraphProbeReplay,
}

impl GraphWorkspaceProbeHistoryReplay {
    /// Borrow the reconstructed workspace document.
    pub const fn workspace(&self) -> &GraphWorkspaceDocument {
        self.workspace.document()
    }

    /// Borrow the byte-for-byte verified workspace encoding.
    pub const fn workspace_encoding(&self) -> &CanonicalGraphWorkspaceEncoding {
        self.workspace.encoding()
    }

    /// Borrow the reconstructed probe document bound to [`Self::workspace`].
    pub const fn probes(&self) -> &GraphProbeDocument {
        self.probes.document()
    }

    /// Borrow the byte-for-byte verified probe encoding.
    pub const fn probe_encoding(&self) -> &CanonicalGraphProbeEncoding {
        self.probes.encoding()
    }

    /// Consume the pair and return its independently replayed carriers.
    pub fn into_parts(self) -> (GraphWorkspaceReplay, GraphProbeReplay) {
        (self.workspace, self.probes)
    }
}

/// Bounded ephemeral history around one exact current ALGW/ALGP pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphWorkspaceProbeHistory {
    limits: GraphWorkspaceProbeHistoryLimits,
    undo: Vec<GraphWorkspaceProbeSnapshot>,
    redo: Vec<GraphWorkspaceProbeSnapshot>,
    retained_bytes: usize,
}

impl GraphWorkspaceProbeHistory {
    /// Construct empty pair history under an explicit retention policy.
    pub fn try_new(
        limits: GraphWorkspaceProbeHistoryLimits,
    ) -> Result<Self, GraphWorkspaceProbeHistoryError> {
        limits.validate()?;
        Ok(Self {
            limits,
            undo: Vec::new(),
            redo: Vec::new(),
            retained_bytes: 0,
        })
    }

    /// Return the retained policy.
    pub const fn limits(&self) -> GraphWorkspaceProbeHistoryLimits {
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

    /// Return combined canonical ALGW and ALGP history bytes.
    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    /// Return whether a prior exact pair exists.
    pub const fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Return whether a later exact pair exists.
    pub const fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Replay the next undo target without changing navigation state.
    pub fn preview_undo(
        &self,
        workspace_admission: GraphWorkspaceLimits,
        graph_admission: GraphLimits,
        probe_admission: GraphProbeLimits,
    ) -> Result<Option<GraphWorkspaceProbeHistoryReplay>, GraphWorkspaceProbeHistoryError> {
        self.undo
            .last()
            .map(|target| {
                replay_snapshot(
                    target,
                    workspace_admission,
                    graph_admission,
                    probe_admission,
                )
            })
            .transpose()
    }

    /// Replay the next redo target without changing navigation state.
    pub fn preview_redo(
        &self,
        workspace_admission: GraphWorkspaceLimits,
        graph_admission: GraphLimits,
        probe_admission: GraphProbeLimits,
    ) -> Result<Option<GraphWorkspaceProbeHistoryReplay>, GraphWorkspaceProbeHistoryError> {
        self.redo
            .last()
            .map(|target| {
                replay_snapshot(
                    target,
                    workspace_admission,
                    graph_admission,
                    probe_admission,
                )
            })
            .transpose()
    }

    /// Record a successfully replaced prior pair and clear the abandoned redo
    /// branch. Oldest undo snapshots are evicted until both bounds hold.
    pub fn record(
        &mut self,
        prior_workspace: &GraphWorkspaceDocument,
        prior_probes: &GraphProbeDocument,
    ) -> Result<(), GraphWorkspaceProbeHistoryError> {
        let prior = GraphWorkspaceProbeSnapshot::capture(prior_workspace, prior_probes)?;
        self.require_snapshot_size(&prior)?;
        let redo_bytes = self.redo.iter().try_fold(0_usize, |total, snapshot| {
            total.checked_add(snapshot.byte_length)
        });
        let redo_bytes = redo_bytes.ok_or(GraphWorkspaceProbeHistoryError::ByteAccounting)?;
        let retained_without_redo = self
            .retained_bytes
            .checked_sub(redo_bytes)
            .ok_or(GraphWorkspaceProbeHistoryError::ByteAccounting)?;
        let retained_with_prior = retained_without_redo
            .checked_add(prior.byte_length)
            .ok_or(GraphWorkspaceProbeHistoryError::ByteAccounting)?;
        self.redo.clear();
        self.retained_bytes = retained_with_prior;
        self.undo.push(prior);
        self.enforce_limits();
        Ok(())
    }

    /// Navigate to the newest prior pair after independently replaying both
    /// artifacts. Failure leaves history and the caller's current pair intact.
    pub fn undo(
        &mut self,
        current_workspace: &GraphWorkspaceDocument,
        current_probes: &GraphProbeDocument,
        workspace_admission: GraphWorkspaceLimits,
        graph_admission: GraphLimits,
        probe_admission: GraphProbeLimits,
    ) -> Result<Option<GraphWorkspaceProbeHistoryReplay>, GraphWorkspaceProbeHistoryError> {
        let Some(target) = self.undo.last() else {
            return Ok(None);
        };
        let replay = replay_snapshot(
            target,
            workspace_admission,
            graph_admission,
            probe_admission,
        )?;
        let target_byte_length = target.byte_length;
        let current = GraphWorkspaceProbeSnapshot::capture(current_workspace, current_probes)?;
        self.require_snapshot_size(&current)?;
        let retained_without_target = self
            .retained_bytes
            .checked_sub(target_byte_length)
            .ok_or(GraphWorkspaceProbeHistoryError::ByteAccounting)?;
        let retained_with_current = retained_without_target
            .checked_add(current.byte_length)
            .ok_or(GraphWorkspaceProbeHistoryError::ByteAccounting)?;
        self.undo.pop().expect("checked nonempty undo stack");
        self.retained_bytes = retained_with_current;
        self.redo.push(current);
        self.enforce_limits();
        Ok(Some(replay))
    }

    /// Navigate to the newest later pair after independently replaying both
    /// artifacts. Failure leaves history and the caller's current pair intact.
    pub fn redo(
        &mut self,
        current_workspace: &GraphWorkspaceDocument,
        current_probes: &GraphProbeDocument,
        workspace_admission: GraphWorkspaceLimits,
        graph_admission: GraphLimits,
        probe_admission: GraphProbeLimits,
    ) -> Result<Option<GraphWorkspaceProbeHistoryReplay>, GraphWorkspaceProbeHistoryError> {
        let Some(target) = self.redo.last() else {
            return Ok(None);
        };
        let replay = replay_snapshot(
            target,
            workspace_admission,
            graph_admission,
            probe_admission,
        )?;
        let target_byte_length = target.byte_length;
        let current = GraphWorkspaceProbeSnapshot::capture(current_workspace, current_probes)?;
        self.require_snapshot_size(&current)?;
        let retained_without_target = self
            .retained_bytes
            .checked_sub(target_byte_length)
            .ok_or(GraphWorkspaceProbeHistoryError::ByteAccounting)?;
        let retained_with_current = retained_without_target
            .checked_add(current.byte_length)
            .ok_or(GraphWorkspaceProbeHistoryError::ByteAccounting)?;
        self.redo.pop().expect("checked nonempty redo stack");
        self.retained_bytes = retained_with_current;
        self.undo.push(current);
        self.enforce_limits();
        Ok(Some(replay))
    }

    /// Clear all ephemeral navigation state without changing the current pair.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.retained_bytes = 0;
    }

    fn require_snapshot_size(
        &self,
        snapshot: &GraphWorkspaceProbeSnapshot,
    ) -> Result<(), GraphWorkspaceProbeHistoryError> {
        if snapshot.byte_length > self.limits.maximum_history_bytes {
            Err(GraphWorkspaceProbeHistoryError::SnapshotTooLarge)
        } else {
            Ok(())
        }
    }

    fn enforce_limits(&mut self) {
        while self.undo.len() > self.limits.maximum_snapshots_per_direction {
            let removed = self.undo.remove(0);
            self.retained_bytes -= removed.byte_length;
        }
        while self.redo.len() > self.limits.maximum_snapshots_per_direction {
            let removed = self.redo.remove(0);
            self.retained_bytes -= removed.byte_length;
        }
        while self.retained_bytes > self.limits.maximum_history_bytes {
            let removed = if !self.undo.is_empty() {
                self.undo.remove(0)
            } else if !self.redo.is_empty() {
                self.redo.remove(0)
            } else {
                break;
            };
            self.retained_bytes -= removed.byte_length;
        }
    }
}

impl Default for GraphWorkspaceProbeHistory {
    fn default() -> Self {
        Self::try_new(GraphWorkspaceProbeHistoryLimits::interactive())
            .expect("interactive graph/probe history limits are nonzero")
    }
}

fn replay_snapshot(
    snapshot: &GraphWorkspaceProbeSnapshot,
    workspace_admission: GraphWorkspaceLimits,
    graph_admission: GraphLimits,
    probe_admission: GraphProbeLimits,
) -> Result<GraphWorkspaceProbeHistoryReplay, GraphWorkspaceProbeHistoryError> {
    let workspace = replay_graph_workspace(
        snapshot.workspace.bytes(),
        workspace_admission,
        graph_admission,
    )?;
    let probes = replay_graph_probes(
        snapshot.probes.bytes(),
        workspace.document(),
        probe_admission,
    )?;
    Ok(GraphWorkspaceProbeHistoryReplay { workspace, probes })
}

/// Rejection at the bounded exact ALGW/ALGP history boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphWorkspaceProbeHistoryError {
    /// A retention policy contained zero.
    ZeroLimit,
    /// One complete ALGW/ALGP snapshot exceeded the history byte budget.
    SnapshotTooLarge,
    /// Checked combined-byte accounting failed.
    ByteAccounting,
    /// Canonical workspace encoding or replay rejected a snapshot.
    Workspace(GraphWorkspaceError),
    /// Canonical probe encoding, replay, or workspace binding rejected a snapshot.
    Probe(GraphProbeError),
}

impl From<GraphWorkspaceError> for GraphWorkspaceProbeHistoryError {
    fn from(value: GraphWorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

impl From<GraphProbeError> for GraphWorkspaceProbeHistoryError {
    fn from(value: GraphProbeError) -> Self {
        Self::Probe(value)
    }
}

impl fmt::Display for GraphWorkspaceProbeHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("graph/probe history policy contains zero"),
            Self::SnapshotTooLarge => {
                formatter.write_str("canonical graph/probe snapshot exceeds history policy")
            }
            Self::ByteAccounting => {
                formatter.write_str("graph/probe history byte accounting failed")
            }
            Self::Workspace(error) => write!(formatter, "graph/probe history workspace: {error}"),
            Self::Probe(error) => write!(formatter, "graph/probe history sidecar: {error}"),
        }
    }
}

impl std::error::Error for GraphWorkspaceProbeHistoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        GraphNodeId, GraphNodePlacement, GraphProbeCapture, GraphProbeDefinition, GraphProbeEdge,
        GraphProbeId, GraphProbeTrigger, RepresentativeControlSignal,
        compile_representative_exact_control_graph,
    };

    fn reference_pair() -> (GraphWorkspaceDocument, GraphProbeDocument) {
        let fixture = compile_representative_exact_control_graph().unwrap();
        let placements = fixture
            .document()
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
        let workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            fixture
                .document()
                .nodes()
                .iter()
                .map(|node| u64::from(node.id().get()))
                .max()
                .unwrap()
                + 1,
            fixture
                .document()
                .wires()
                .iter()
                .map(|wire| u64::from(wire.id().get()))
                .max()
                .unwrap()
                + 1,
            fixture.document().clone(),
            placements,
        )
        .unwrap();
        let source = RepresentativeControlSignal::MeasurementWithinRange.endpoint();
        let probes = GraphProbeDocument::try_new(
            GraphProbeLimits::interactive(),
            1,
            2,
            Some(GraphProbeTrigger::new(
                GraphProbeId::new(1),
                GraphProbeEdge::Falling,
                2,
                2,
            )),
            &workspace,
            vec![GraphProbeDefinition::new(
                GraphProbeId::new(1),
                "measurement-in-range",
                source,
                GraphProbeCapture::new(32, 1),
            )],
        )
        .unwrap();
        (workspace, probes)
    }

    fn moved_pair(
        workspace: &GraphWorkspaceDocument,
        probes: &GraphProbeDocument,
        delta: i32,
    ) -> (GraphWorkspaceDocument, GraphProbeDocument) {
        let mut moved = workspace.clone();
        let id = GraphNodeId::new(1);
        let placement = moved.placement(id).unwrap();
        moved
            .move_node(id, placement.x() + delta, placement.y())
            .unwrap();
        let mut rebound = probes.clone();
        rebound.replace_workspace(&moved).unwrap();
        (moved, rebound)
    }

    #[test]
    fn undo_redo_replays_the_exact_bound_pair() {
        let (initial_workspace, initial_probes) = reference_pair();
        let (moved_workspace, moved_probes) = moved_pair(&initial_workspace, &initial_probes, 7);
        let initial_bytes = encode_graph_workspace(&initial_workspace)
            .unwrap()
            .bytes()
            .len()
            + encode_graph_probes(&initial_probes).unwrap().bytes().len();
        let moved_bytes = encode_graph_workspace(&moved_workspace)
            .unwrap()
            .bytes()
            .len()
            + encode_graph_probes(&moved_probes).unwrap().bytes().len();
        let mut history = GraphWorkspaceProbeHistory::default();
        history.record(&initial_workspace, &initial_probes).unwrap();
        assert_eq!(history.retained_bytes(), initial_bytes);

        let undone = history
            .undo(
                &moved_workspace,
                &moved_probes,
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
                GraphProbeLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(undone.workspace(), &initial_workspace);
        assert_eq!(undone.probes(), &initial_probes);
        assert_eq!(history.retained_bytes(), moved_bytes);
        assert_eq!((history.undo_len(), history.redo_len()), (0, 1));

        let redone = history
            .redo(
                undone.workspace(),
                undone.probes(),
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
                GraphProbeLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(redone.workspace(), &moved_workspace);
        assert_eq!(redone.probes(), &moved_probes);
        assert_eq!((history.undo_len(), history.redo_len()), (1, 0));
    }

    #[test]
    fn pair_record_rejects_mismatch_and_new_edits_clear_redo() {
        let (initial_workspace, initial_probes) = reference_pair();
        let (moved_workspace, moved_probes) = moved_pair(&initial_workspace, &initial_probes, 3);
        let mut history = GraphWorkspaceProbeHistory::default();
        assert!(matches!(
            history.record(&moved_workspace, &initial_probes),
            Err(GraphWorkspaceProbeHistoryError::Probe(
                GraphProbeError::WorkspaceIdentityMismatch { .. }
            ))
        ));
        assert_eq!((history.undo_len(), history.redo_len()), (0, 0));

        history.record(&initial_workspace, &initial_probes).unwrap();
        let undone = history
            .undo(
                &moved_workspace,
                &moved_probes,
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
                GraphProbeLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(history.redo_len(), 1);
        history.record(undone.workspace(), undone.probes()).unwrap();
        assert_eq!((history.undo_len(), history.redo_len()), (1, 0));
    }

    #[test]
    fn oldest_complete_pairs_are_evicted_as_one_snapshot() {
        let (initial_workspace, initial_probes) = reference_pair();
        let (first_workspace, first_probes) = moved_pair(&initial_workspace, &initial_probes, 2);
        let mut history = GraphWorkspaceProbeHistory::try_new(GraphWorkspaceProbeHistoryLimits {
            maximum_snapshots_per_direction: 1,
            maximum_history_bytes: 64 * 1024 * 1024,
        })
        .unwrap();
        history.record(&initial_workspace, &initial_probes).unwrap();
        history.record(&first_workspace, &first_probes).unwrap();
        assert_eq!((history.undo_len(), history.redo_len()), (1, 0));
        let retained = history
            .preview_undo(
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
                GraphProbeLimits::interactive(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(retained.workspace(), &first_workspace);
        assert_eq!(retained.probes(), &first_probes);
    }

    #[test]
    fn tighter_replay_policy_and_history_limits_fail_transactionally() {
        assert_eq!(
            GraphWorkspaceProbeHistory::try_new(GraphWorkspaceProbeHistoryLimits {
                maximum_snapshots_per_direction: 0,
                maximum_history_bytes: 1,
            }),
            Err(GraphWorkspaceProbeHistoryError::ZeroLimit)
        );
        let (initial_workspace, initial_probes) = reference_pair();
        let (moved_workspace, moved_probes) = moved_pair(&initial_workspace, &initial_probes, 5);
        let pair_bytes = encode_graph_workspace(&initial_workspace)
            .unwrap()
            .bytes()
            .len()
            + encode_graph_probes(&initial_probes).unwrap().bytes().len();
        let mut too_small = GraphWorkspaceProbeHistory::try_new(GraphWorkspaceProbeHistoryLimits {
            maximum_snapshots_per_direction: 1,
            maximum_history_bytes: pair_bytes - 1,
        })
        .unwrap();
        assert_eq!(
            too_small.record(&initial_workspace, &initial_probes),
            Err(GraphWorkspaceProbeHistoryError::SnapshotTooLarge)
        );

        let mut history = GraphWorkspaceProbeHistory::default();
        history.record(&initial_workspace, &initial_probes).unwrap();
        let retained = history.clone();
        let tight_probes = GraphProbeLimits {
            maximum_probes: 1,
            ..GraphProbeLimits::interactive()
        };
        assert!(matches!(
            history.undo(
                &moved_workspace,
                &moved_probes,
                GraphWorkspaceLimits::interactive(),
                GraphLimits::interactive(),
                tight_probes,
            ),
            Err(GraphWorkspaceProbeHistoryError::Probe(
                GraphProbeError::LimitExceeded("embedded limits")
            ))
        ));
        assert_eq!(history, retained);
    }
}
