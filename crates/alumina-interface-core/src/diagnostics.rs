//! Owned, capability-bound diagnostic evidence for native/WASM presentation.
//!
//! This module independently decodes canonical records and reconciles every
//! typed selector against the complete board capability. It does not infer
//! acquisition or command authority from descriptive resource presence.

use core::fmt;

use alumina_board::{
    DiagnosticObservationKind, DigitalCaptureSourceKind, DigitalCaptureTriggerSet, ResourceId,
};
use alumina_capability::CapabilityIdentity;
use alumina_diagnostics::{
    CaptureId, CaptureQualityFlags, DIGITAL_CAPTURE_VERSION, DiagnosticContext, DiagnosticError,
    DiagnosticLimits, DigitalAcquisitionSource, DigitalCaptureChannel, DigitalCaptureFlags,
    DigitalCaptureState, DigitalLevel, DigitalTransition, DigitalTriggerCondition, OverviewFlags,
    ResourceOverviewSample, decode_digital_capture, decode_resource_overview,
};
use alumina_protocol::DeviceCycle;

use crate::BoardExplorerSnapshot;

/// Immutable decoded overview/capture pair bound to one exact board package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticExplorerSnapshot {
    context: DiagnosticContext,
    overview_flags: OverviewFlags,
    overview_snapshot_cycle: DeviceCycle,
    overview_sequence: u64,
    overview_bytes: usize,
    overview_samples: Vec<ResourceOverviewSample>,
    capture_flags: DigitalCaptureFlags,
    capture_id: CaptureId,
    capture_start_cycle: DeviceCycle,
    capture_end_cycle_exclusive: DeviceCycle,
    requested_pretrigger_cycles: u64,
    requested_posttrigger_cycles: u64,
    trigger_cycle: DeviceCycle,
    trigger_channel_index: u16,
    trigger_condition: DigitalTriggerCondition,
    trigger_transition_index: u32,
    capture_state: DigitalCaptureState,
    transition_capacity: u32,
    retained_event_stride: u32,
    capture_quality_flags: CaptureQualityFlags,
    capture_bytes: usize,
    capture_channels: Vec<DigitalCaptureChannel>,
    transitions: Vec<DigitalTransition>,
}

impl DiagnosticExplorerSnapshot {
    /// Shared device, boot, package, configuration, and clock identity.
    pub const fn context(&self) -> DiagnosticContext {
        self.context
    }

    /// Overview provenance flags.
    pub const fn overview_flags(&self) -> OverviewFlags {
        self.overview_flags
    }

    /// Snapshot assembly cycle and monotonic sequence.
    pub const fn overview_position(&self) -> (DeviceCycle, u64) {
        (self.overview_snapshot_cycle, self.overview_sequence)
    }

    /// Exact canonical overview byte length.
    pub const fn overview_bytes(&self) -> usize {
        self.overview_bytes
    }

    /// Explicit overview records; omission conveys no state.
    pub fn overview_samples(&self) -> &[ResourceOverviewSample] {
        &self.overview_samples
    }

    /// Finds one explicit overview record.
    pub fn overview_sample(&self, resource: ResourceId) -> Option<ResourceOverviewSample> {
        self.overview_samples
            .iter()
            .copied()
            .find(|sample| sample.resource == resource)
    }

    /// Computes snapshot age in exact device cycles for one explicit record.
    pub fn overview_age_cycles(&self, resource: ResourceId) -> Option<u64> {
        self.overview_sample(resource).map(|sample| {
            self.overview_snapshot_cycle
                .0
                .checked_sub(sample.captured_cycle.0)
                .expect("canonical decoder rejected future samples")
        })
    }

    /// Capture provenance flags and identity.
    pub const fn capture_identity(&self) -> (DigitalCaptureFlags, CaptureId) {
        (self.capture_flags, self.capture_id)
    }

    /// Exact retained `[start, end)` interval.
    pub const fn capture_window(&self) -> (DeviceCycle, DeviceCycle) {
        (self.capture_start_cycle, self.capture_end_cycle_exclusive)
    }

    /// Requested pretrigger and posttrigger cycle counts.
    pub const fn requested_capture_window(&self) -> (u64, u64) {
        (
            self.requested_pretrigger_cycles,
            self.requested_posttrigger_cycles,
        )
    }

    /// Actual trigger facts.
    pub const fn trigger(&self) -> (DeviceCycle, u16, DigitalTriggerCondition, u32) {
        (
            self.trigger_cycle,
            self.trigger_channel_index,
            self.trigger_condition,
            self.trigger_transition_index,
        )
    }

    /// Terminal acquisition state.
    pub const fn capture_state(&self) -> DigitalCaptureState {
        self.capture_state
    }

    /// Fixed capacity and retained-event stride.
    pub const fn capture_retention(&self) -> (u32, u32) {
        (self.transition_capacity, self.retained_event_stride)
    }

    /// Capture-wide loss and confidence annotations.
    pub const fn capture_quality_flags(&self) -> CaptureQualityFlags {
        self.capture_quality_flags
    }

    /// Exact canonical digital-capture byte length.
    pub const fn capture_bytes(&self) -> usize {
        self.capture_bytes
    }

    /// Explicit capture channels in canonical typed-resource order.
    pub fn capture_channels(&self) -> &[DigitalCaptureChannel] {
        &self.capture_channels
    }

    /// Retained transitions in canonical `(offset, channel)` order.
    pub fn transitions(&self) -> &[DigitalTransition] {
        &self.transitions
    }

    /// Finds a channel's capture-local index.
    pub fn capture_channel_index(&self, resource: ResourceId) -> Option<u16> {
        self.capture_channels
            .iter()
            .position(|channel| channel.resource == resource)
            .and_then(|index| u16::try_from(index).ok())
    }

    /// Resolves the logical level at an exact offset in the retained window.
    pub fn digital_level_at(
        &self,
        resource: ResourceId,
        offset_cycles: u64,
    ) -> Option<DigitalLevel> {
        let channel_index = self.capture_channel_index(resource)?;
        let channel = self.capture_channels.get(usize::from(channel_index))?;
        let mut level = channel.initial_level;
        for transition in &self.transitions {
            if transition.offset_cycles > offset_cycles {
                break;
            }
            if transition.channel_index == channel_index {
                level = transition.level;
            }
        }
        Some(level)
    }
}

/// Failure while constructing a board-reconciled owned diagnostic view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticExplorerError {
    /// Overview bytes failed canonical decoding.
    Overview(DiagnosticError),
    /// Capture bytes failed canonical decoding.
    Capture(DiagnosticError),
    /// Evidence does not identify the supplied complete board capability.
    CapabilityIdentity {
        /// Board explorer's exact capability identity.
        expected: CapabilityIdentity,
        /// Identity carried by the rejected diagnostic context.
        received: CapabilityIdentity,
    },
    /// Overview and capture belong to different devices, boots, configs, or clocks.
    ContextMismatch,
    /// Simulation/physical provenance differs between overview and capture.
    ProvenanceMismatch,
    /// Evidence names a typed resource absent from the bound board package.
    UnknownResource(ResourceId),
    /// Overview evidence names a resource outside the passive observation palette.
    UnadmittedOverviewResource(ResourceId),
    /// Capture shape, timing, trigger, or byte facts exceed the provider catalog.
    CaptureCapability,
    /// Capture evidence names a resource outside the acquisition palette.
    UnadmittedCaptureResource(ResourceId),
    /// Capture evidence reports a source other than the catalogued acquisition path.
    CaptureSource(ResourceId),
    /// Bounded owned UI state could not be allocated.
    Allocation,
}

impl fmt::Display for DiagnosticExplorerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "diagnostic explorer rejected evidence: {self:?}")
    }
}

impl std::error::Error for DiagnosticExplorerError {}

/// Decode, cross-reconcile, and own one overview/capture pair for presentation.
pub fn build_diagnostic_explorer_snapshot(
    board: &BoardExplorerSnapshot,
    overview_bytes: &[u8],
    capture_bytes: &[u8],
    limits: DiagnosticLimits,
) -> Result<DiagnosticExplorerSnapshot, DiagnosticExplorerError> {
    let overview = decode_resource_overview(overview_bytes, limits)
        .map_err(DiagnosticExplorerError::Overview)?;
    let capture =
        decode_digital_capture(capture_bytes, limits).map_err(DiagnosticExplorerError::Capture)?;
    if overview.context().capability != board.identity() {
        return Err(DiagnosticExplorerError::CapabilityIdentity {
            expected: board.identity(),
            received: overview.context().capability,
        });
    }
    if capture.context().capability != board.identity() {
        return Err(DiagnosticExplorerError::CapabilityIdentity {
            expected: board.identity(),
            received: capture.context().capability,
        });
    }
    if overview.context() != capture.context() {
        return Err(DiagnosticExplorerError::ContextMismatch);
    }
    if overview.flags().contains(OverviewFlags::SIMULATED)
        != capture.flags().contains(DigitalCaptureFlags::SIMULATED)
    {
        return Err(DiagnosticExplorerError::ProvenanceMismatch);
    }
    validate_capture_capability(board, capture, capture_bytes.len())?;

    let mut overview_samples = Vec::new();
    overview_samples
        .try_reserve_exact(overview.sample_count())
        .map_err(|_| DiagnosticExplorerError::Allocation)?;
    for sample in overview.samples() {
        require_overview_resource(board, sample.resource)?;
        overview_samples.push(sample);
    }

    let mut capture_channels = Vec::new();
    capture_channels
        .try_reserve_exact(capture.channel_count())
        .map_err(|_| DiagnosticExplorerError::Allocation)?;
    for channel in capture.channels() {
        require_capture_resource(board, channel)?;
        capture_channels.push(channel);
    }

    let mut transitions = Vec::new();
    transitions
        .try_reserve_exact(capture.transition_count())
        .map_err(|_| DiagnosticExplorerError::Allocation)?;
    transitions.extend(capture.transitions());

    let (overview_snapshot_cycle, overview_sequence) =
        (overview.snapshot_cycle(), overview.sequence());
    let (capture_flags, capture_id) = (capture.flags(), capture.capture_id());
    let (capture_start_cycle, capture_end_cycle_exclusive) = capture.cycle_window();
    let (requested_pretrigger_cycles, requested_posttrigger_cycles) =
        capture.requested_window_cycles();
    let (trigger_cycle, trigger_channel_index, trigger_condition, trigger_transition_index) =
        capture.trigger();
    let (transition_capacity, retained_event_stride) = capture.retention();
    Ok(DiagnosticExplorerSnapshot {
        context: overview.context(),
        overview_flags: overview.flags(),
        overview_snapshot_cycle,
        overview_sequence,
        overview_bytes: overview_bytes.len(),
        overview_samples,
        capture_flags,
        capture_id,
        capture_start_cycle,
        capture_end_cycle_exclusive,
        requested_pretrigger_cycles,
        requested_posttrigger_cycles,
        trigger_cycle,
        trigger_channel_index,
        trigger_condition,
        trigger_transition_index,
        capture_state: capture.state(),
        transition_capacity,
        retained_event_stride,
        capture_quality_flags: capture.quality_flags(),
        capture_bytes: capture_bytes.len(),
        capture_channels,
        transitions,
    })
}

fn validate_capture_capability(
    board: &BoardExplorerSnapshot,
    capture: alumina_diagnostics::DigitalCaptureView<'_>,
    capture_bytes: usize,
) -> Result<(), DiagnosticExplorerError> {
    let provider = board.digital_capture();
    let transition_capacity = capture.retention().0;
    let (pretrigger, posttrigger) = capture.requested_window_cycles();
    let duration = pretrigger
        .checked_add(posttrigger)
        .ok_or(DiagnosticExplorerError::CaptureCapability)?;
    let maximum_pretrigger = cycles_for_micros_floor(
        capture.context().clock_frequency_hz,
        provider.maximum_pretrigger_micros,
    )?;
    let maximum_duration = cycles_for_micros_floor(
        capture.context().clock_frequency_hz,
        provider.maximum_duration_micros,
    )?;
    let maximum_record_bytes = usize::try_from(provider.record_bytes)
        .map_err(|_| DiagnosticExplorerError::CaptureCapability)?;
    let maximum_transitions = usize::try_from(provider.maximum_transitions)
        .map_err(|_| DiagnosticExplorerError::CaptureCapability)?;
    let trigger_bit = match capture.trigger().2 {
        DigitalTriggerCondition::Immediate => DigitalCaptureTriggerSet::IMMEDIATE,
        DigitalTriggerCondition::Rising => DigitalCaptureTriggerSet::RISING,
        DigitalTriggerCondition::Falling => DigitalCaptureTriggerSet::FALLING,
        DigitalTriggerCondition::Either => DigitalCaptureTriggerSet::EITHER,
    };
    if !provider.is_implemented()
        || provider.schema_version != DIGITAL_CAPTURE_VERSION
        || capture_bytes > maximum_record_bytes
        || capture.channel_count() > usize::from(provider.maximum_channels)
        || capture.transition_count() > maximum_transitions
        || transition_capacity > provider.maximum_transitions
        || pretrigger > maximum_pretrigger
        || duration > maximum_duration
        || !provider.trigger_kinds.contains(trigger_bit)
    {
        return Err(DiagnosticExplorerError::CaptureCapability);
    }
    Ok(())
}

fn require_capture_resource(
    board: &BoardExplorerSnapshot,
    channel: DigitalCaptureChannel,
) -> Result<(), DiagnosticExplorerError> {
    let Some(resource) = board.resource(channel.resource) else {
        return Err(DiagnosticExplorerError::UnknownResource(channel.resource));
    };
    let Some(expected) = resource.digital_capture() else {
        return Err(DiagnosticExplorerError::UnadmittedCaptureResource(
            channel.resource,
        ));
    };
    if acquisition_source(channel.source) != Some(expected.source) {
        return Err(DiagnosticExplorerError::CaptureSource(channel.resource));
    }
    Ok(())
}

fn cycles_for_micros_floor(frequency_hz: u64, micros: u32) -> Result<u64, DiagnosticExplorerError> {
    let cycles = u128::from(frequency_hz)
        .checked_mul(u128::from(micros))
        .ok_or(DiagnosticExplorerError::CaptureCapability)?
        / 1_000_000;
    u64::try_from(cycles).map_err(|_| DiagnosticExplorerError::CaptureCapability)
}

const fn acquisition_source(source: DigitalAcquisitionSource) -> Option<DigitalCaptureSourceKind> {
    match source {
        DigitalAcquisitionSource::Simulated => Some(DigitalCaptureSourceKind::Simulated),
        DigitalAcquisitionSource::Rmt => Some(DigitalCaptureSourceKind::Rmt),
        DigitalAcquisitionSource::Pcnt => Some(DigitalCaptureSourceKind::Pcnt),
        DigitalAcquisitionSource::Dma => Some(DigitalCaptureSourceKind::Dma),
        DigitalAcquisitionSource::Software => Some(DigitalCaptureSourceKind::Software),
        DigitalAcquisitionSource::ExternalAnalyzer => None,
    }
}

fn require_overview_resource(
    board: &BoardExplorerSnapshot,
    resource: ResourceId,
) -> Result<(), DiagnosticExplorerError> {
    let Some(resource_descriptor) = board.resource(resource) else {
        return Err(DiagnosticExplorerError::UnknownResource(resource));
    };
    if resource_descriptor
        .diagnostic_observations()
        .iter()
        .any(|observation| observation.observation == DiagnosticObservationKind::StableBooleanInput)
    {
        Ok(())
    } else {
        Err(DiagnosticExplorerError::UnadmittedOverviewResource(
            resource,
        ))
    }
}

#[cfg(test)]
mod tests {
    use alumina_capability::{
        BoardCapabilityLimits, MAX_CAPABILITY_CHUNK_BYTES, calculate_identity, read_verified_range,
    };
    use alumina_diagnostics::{
        DIGITAL_CAPTURE_CHANNEL_BYTES, DIGITAL_CAPTURE_HEADER_BYTES, DigitalAcquisitionSource,
        DigitalCaptureFlags, OverviewFlags, RESOURCE_OVERVIEW_HEADER_BYTES,
        RESOURCE_OVERVIEW_SAMPLE_BYTES, ResourceValue, SampleProvenance,
    };
    use alumina_sim::diagnostics::tinybee_diagnostic_fixture;

    use super::*;
    use crate::build_board_explorer_snapshot;

    fn board() -> BoardExplorerSnapshot {
        let package = alumina_sim::capability::package();
        let identity = calculate_identity(&package).unwrap();
        let mut document = vec![0_u8; usize::try_from(identity.byte_len).unwrap()];
        let mut offset = 0_u32;
        while offset < identity.byte_len {
            let mut chunk = [0_u8; MAX_CAPABILITY_CHUNK_BYTES];
            let read = read_verified_range(&package, offset, &mut chunk).unwrap();
            let start = usize::try_from(offset).unwrap();
            let count = usize::from(read.byte_len);
            document[start..start + count].copy_from_slice(&chunk[..count]);
            offset += u32::from(read.byte_len);
        }
        build_board_explorer_snapshot(&document, identity, BoardCapabilityLimits::interactive())
            .unwrap()
    }

    #[test]
    fn fixture_reconciles_to_board_and_cross_links_selected_resource() {
        let board = board();
        let fixture = tinybee_diagnostic_fixture().unwrap();
        let diagnostics = build_diagnostic_explorer_snapshot(
            &board,
            fixture.overview_bytes(),
            fixture.digital_capture_bytes(),
            DiagnosticLimits::interactive(),
        )
        .unwrap();
        assert_eq!(diagnostics.context().capability, board.identity());
        assert!(
            diagnostics
                .overview_flags()
                .contains(OverviewFlags::SIMULATED)
        );
        assert!(
            diagnostics
                .capture_identity()
                .0
                .contains(DigitalCaptureFlags::SIMULATED)
        );
        assert_eq!(diagnostics.overview_samples().len(), 4);
        assert_eq!(diagnostics.capture_channels().len(), 4);
        assert_eq!(diagnostics.transitions().len(), 14);
        assert_eq!(
            diagnostics
                .overview_sample(ResourceId::Gpio(33))
                .unwrap()
                .value,
            ResourceValue::Boolean(false)
        );
        assert_eq!(
            diagnostics.overview_age_cycles(ResourceId::Gpio(33)),
            Some(50)
        );
        assert_eq!(
            diagnostics.capture_channel_index(ResourceId::Gpio(33)),
            Some(2)
        );
        assert_eq!(
            diagnostics.digital_level_at(ResourceId::Gpio(33), 499),
            Some(DigitalLevel::Low)
        );
        assert_eq!(
            diagnostics.digital_level_at(ResourceId::Gpio(33), 500),
            Some(DigitalLevel::High)
        );
        assert_eq!(
            diagnostics.digital_level_at(ResourceId::Gpio(33), 620),
            Some(DigitalLevel::Low)
        );
    }

    #[test]
    fn context_substitution_and_unknown_board_resources_fail_closed() {
        let board = board();
        let fixture = tinybee_diagnostic_fixture().unwrap();
        let mut overview = fixture.overview_bytes().to_vec();
        overview[88] ^= 1;
        assert_eq!(
            build_diagnostic_explorer_snapshot(
                &board,
                &overview,
                fixture.digital_capture_bytes(),
                DiagnosticLimits::interactive(),
            ),
            Err(DiagnosticExplorerError::ContextMismatch)
        );

        overview.copy_from_slice(fixture.overview_bytes());
        let last_resource = 160 + 3 * 40;
        overview[last_resource..last_resource + 4].copy_from_slice(
            &alumina_capability::encode_resource_id(ResourceId::Gpio(63)),
        );
        assert_eq!(
            build_diagnostic_explorer_snapshot(
                &board,
                &overview,
                fixture.digital_capture_bytes(),
                DiagnosticLimits::interactive(),
            ),
            Err(DiagnosticExplorerError::UnknownResource(ResourceId::Gpio(
                63
            )))
        );

        overview.copy_from_slice(fixture.overview_bytes());
        overview[last_resource..last_resource + 4].copy_from_slice(
            &alumina_capability::encode_resource_id(ResourceId::Gpio(34)),
        );
        assert_eq!(
            build_diagnostic_explorer_snapshot(
                &board,
                &overview,
                fixture.digital_capture_bytes(),
                DiagnosticLimits::interactive(),
            ),
            Err(DiagnosticExplorerError::UnadmittedOverviewResource(
                ResourceId::Gpio(34)
            ))
        );

        let mut wrong_source = fixture.digital_capture_bytes().to_vec();
        wrong_source[10..12].copy_from_slice(&0_u16.to_le_bytes());
        for channel in 0..4 {
            wrong_source
                [DIGITAL_CAPTURE_HEADER_BYTES + channel * DIGITAL_CAPTURE_CHANNEL_BYTES + 5] =
                DigitalAcquisitionSource::Rmt as u8;
        }
        let mut measured_overview = fixture.overview_bytes().to_vec();
        measured_overview[10..12].copy_from_slice(&0_u16.to_le_bytes());
        for sample in 0..4 {
            measured_overview
                [RESOURCE_OVERVIEW_HEADER_BYTES + sample * RESOURCE_OVERVIEW_SAMPLE_BYTES + 4] =
                SampleProvenance::Measured as u8;
        }
        assert_eq!(
            build_diagnostic_explorer_snapshot(
                &board,
                &measured_overview,
                &wrong_source,
                DiagnosticLimits::interactive(),
            ),
            Err(DiagnosticExplorerError::CaptureSource(ResourceId::Gpio(22)))
        );
    }
}
