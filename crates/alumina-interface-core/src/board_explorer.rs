//! Capability-derived, board-name-independent diagnostic explorer model.
//!
//! A complete capability document is descriptive authority for the board image
//! that emitted it. Its graph-resource subsection is intentionally narrower:
//! only entries in that subsection represent graph operations. This module
//! preserves that distinction while constructing bounded owned UI state.

use core::fmt;

use alumina_board::{
    Chip, GraphResourceDescriptor, NormalizedPoint, OwnerDomain, Qualification, ResourceDescriptor,
    ResourceId,
};
use alumina_capability::{
    BoardCapabilityLimits, CapabilityDocumentError, CapabilityIdentity, decode_board_capability,
};
use alumina_protocol::Digest;

/// One descriptive board resource with its aliases and separately admitted
/// graph operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoardExplorerResource {
    descriptor: ResourceDescriptor,
    aliases: Vec<String>,
    graph_accesses: Vec<GraphResourceDescriptor>,
}

impl BoardExplorerResource {
    /// Immutable ownership, safe-state, and hazard facts.
    pub const fn descriptor(&self) -> ResourceDescriptor {
        self.descriptor
    }

    /// Canonical and silkscreen aliases targeting this exact typed resource.
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    /// Explicit graph operations admitted by the exact firmware image.
    pub fn graph_accesses(&self) -> &[GraphResourceDescriptor] {
        &self.graph_accesses
    }

    /// Whether any graph operation is admitted for this resource.
    pub fn is_graph_addressable(&self) -> bool {
        !self.graph_accesses.is_empty()
    }
}

/// One decoded, typed polygon over a digest-bound licensed board visual.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoardExplorerHotspot {
    id: String,
    resource: ResourceId,
    polygon: Vec<NormalizedPoint>,
}

impl BoardExplorerHotspot {
    /// Stable hotspot ID within its visual.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Typed resource represented by the polygon.
    pub const fn resource(&self) -> ResourceId {
        self.resource
    }

    /// Reviewed normalized polygon points in the 0–10,000 image plane.
    pub fn polygon(&self) -> &[NormalizedPoint] {
        &self.polygon
    }
}

/// One decoded licensed board visual and its exact hotspot set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoardExplorerVisual {
    id: String,
    asset_path: String,
    media_type: String,
    pixel_width: u32,
    pixel_height: u32,
    asset_digest: Digest,
    license: String,
    attribution: String,
    hotspots: Vec<BoardExplorerHotspot>,
}

impl BoardExplorerVisual {
    /// Stable visual ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Repository-relative raster asset path.
    pub fn asset_path(&self) -> &str {
        &self.asset_path
    }

    /// Exact raster MIME type.
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Reviewed raster dimensions in pixels.
    pub const fn pixel_dimensions(&self) -> (u32, u32) {
        (self.pixel_width, self.pixel_height)
    }

    /// SHA-256 over the exact raster bytes.
    pub const fn asset_digest(&self) -> Digest {
        self.asset_digest
    }

    /// SPDX expression for the visual asset.
    pub fn license(&self) -> &str {
        &self.license
    }

    /// Required source/photographer attribution.
    pub fn attribution(&self) -> &str {
        &self.attribution
    }

    /// Reviewed typed hotspot polygons.
    pub fn hotspots(&self) -> &[BoardExplorerHotspot] {
        &self.hotspots
    }
}

/// Immutable owned board-explorer state derived only from a complete canonical
/// capability document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoardExplorerSnapshot {
    identity: CapabilityIdentity,
    board_id: String,
    revision: String,
    chip: Chip,
    application_cores: u8,
    qualification: Qualification,
    armable: bool,
    flash_bytes: u64,
    internal_sram_bytes: u64,
    psram_bytes: u64,
    service_core: u8,
    realtime_core: u8,
    resources: Vec<BoardExplorerResource>,
    alias_count: usize,
    graph_resource_count: usize,
    bus_count: usize,
    device_count: usize,
    flash_region_count: usize,
    clock_count: usize,
    electrical_constraint_count: usize,
    interrupt_count: usize,
    safe_output_image_count: usize,
    visuals: Vec<BoardExplorerVisual>,
    hil_requirement_count: usize,
}

impl BoardExplorerSnapshot {
    /// Exact identity of every decoded capability byte.
    pub const fn identity(&self) -> CapabilityIdentity {
        self.identity
    }

    /// Stable board/revision ID.
    pub fn board_id(&self) -> &str {
        &self.board_id
    }

    /// Human-readable physical-revision evidence string.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// Application MCU family.
    pub const fn chip(&self) -> Chip {
        self.chip
    }

    /// Number of application cores.
    pub const fn application_cores(&self) -> u8 {
        self.application_cores
    }

    /// Evidence level of the exact board package.
    pub const fn qualification(&self) -> Qualification {
        self.qualification
    }

    /// Whether the exact package advertises arming eligibility.
    pub const fn armable(&self) -> bool {
        self.armable
    }

    /// Flash, internal SRAM, and external PSRAM capacities in bytes.
    pub const fn memory_bytes(&self) -> (u64, u64, u64) {
        (self.flash_bytes, self.internal_sram_bytes, self.psram_bytes)
    }

    /// Fixed service and real-time core indices.
    pub const fn core_assignment(&self) -> (u8, u8) {
        (self.service_core, self.realtime_core)
    }

    /// Complete descriptive typed-resource inventory.
    pub fn resources(&self) -> &[BoardExplorerResource] {
        &self.resources
    }

    /// Total canonical aliases before grouping by resource.
    pub const fn alias_count(&self) -> usize {
        self.alias_count
    }

    /// Total explicitly graph-addressable resource records.
    pub const fn graph_resource_count(&self) -> usize {
        self.graph_resource_count
    }

    /// Counts for buses, devices, flash regions, clocks, electrical
    /// constraints, interrupts, and shifted-output safe images.
    pub const fn supporting_section_counts(&self) -> [usize; 7] {
        [
            self.bus_count,
            self.device_count,
            self.flash_region_count,
            self.clock_count,
            self.electrical_constraint_count,
            self.interrupt_count,
            self.safe_output_image_count,
        ]
    }

    /// Digest-bound licensed visuals and reviewed typed hotspot polygons.
    pub fn visuals(&self) -> &[BoardExplorerVisual] {
        &self.visuals
    }

    /// Number of HIL requirements in the capability ledger.
    pub const fn hil_requirement_count(&self) -> usize {
        self.hil_requirement_count
    }

    /// Find one exact typed resource.
    pub fn resource(&self, id: ResourceId) -> Option<&BoardExplorerResource> {
        self.resources
            .iter()
            .find(|resource| resource.descriptor.id == id)
    }

    /// Count resources by executor ownership, hazard, and graph access.
    pub fn resource_summary(&self) -> BoardExplorerResourceSummary {
        let mut summary = BoardExplorerResourceSummary::default();
        for resource in &self.resources {
            match resource.descriptor.owner {
                OwnerDomain::Service => summary.service += 1,
                OwnerDomain::Realtime => summary.realtime += 1,
            }
            summary.hazardous += usize::from(resource.descriptor.hazardous_output);
            summary.graph_addressable += usize::from(resource.is_graph_addressable());
        }
        summary
    }
}

/// Aggregate counts used by board-explorer filters and review summaries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BoardExplorerResourceSummary {
    /// Service-owned descriptive resources.
    pub service: usize,
    /// Real-time-owned descriptive resources.
    pub realtime: usize,
    /// Resources whose board record marks them as hazardous outputs.
    pub hazardous: usize,
    /// Resources with at least one explicit graph access record.
    pub graph_addressable: usize,
}

/// Failure while deriving bounded owned board-explorer state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoardExplorerError {
    /// The complete document failed independent canonical decoding.
    Capability(CapabilityDocumentError),
    /// The expected identity did not identify the complete supplied bytes.
    IdentityMismatch {
        /// Identity supplied by the caller's trusted context.
        expected: CapabilityIdentity,
        /// Identity calculated over the complete document.
        received: CapabilityIdentity,
    },
    /// Bounded owned UI state could not be allocated.
    Allocation,
}

impl fmt::Display for BoardExplorerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capability(error) => {
                write!(formatter, "board capability document is invalid: {error:?}")
            }
            Self::IdentityMismatch { .. } => formatter
                .write_str("expected capability identity does not identify the supplied bytes"),
            Self::Allocation => formatter.write_str("board explorer allocation failed"),
        }
    }
}

impl std::error::Error for BoardExplorerError {}

/// Build board-name-independent explorer state from one complete capability
/// document after checking its caller-supplied expected identity.
pub fn build_board_explorer_snapshot(
    document: &[u8],
    expected_identity: CapabilityIdentity,
    limits: BoardCapabilityLimits,
) -> Result<BoardExplorerSnapshot, BoardExplorerError> {
    let capability =
        decode_board_capability(document, limits).map_err(BoardExplorerError::Capability)?;
    if capability.identity() != expected_identity {
        return Err(BoardExplorerError::IdentityMismatch {
            expected: expected_identity,
            received: capability.identity(),
        });
    }

    let mut aliases = Vec::new();
    aliases
        .try_reserve_exact(capability.alias_count())
        .map_err(|_| BoardExplorerError::Allocation)?;
    aliases.extend(capability.aliases());
    let mut graph_resources = Vec::new();
    graph_resources
        .try_reserve_exact(capability.graph().resource_count())
        .map_err(|_| BoardExplorerError::Allocation)?;
    graph_resources.extend(capability.graph().resources());
    let mut resources = Vec::new();
    resources
        .try_reserve_exact(capability.resource_count())
        .map_err(|_| BoardExplorerError::Allocation)?;
    for descriptor in capability.resources() {
        let alias_count = aliases
            .iter()
            .filter(|alias| alias.resource == descriptor.id)
            .count();
        let mut resource_aliases = Vec::new();
        resource_aliases
            .try_reserve_exact(alias_count)
            .map_err(|_| BoardExplorerError::Allocation)?;
        for alias in aliases
            .iter()
            .filter(|alias| alias.resource == descriptor.id)
        {
            resource_aliases.push(try_owned(alias.name)?);
        }
        let access_count = graph_resources
            .iter()
            .filter(|access| access.resource == descriptor.id)
            .count();
        let mut graph_accesses = Vec::new();
        graph_accesses
            .try_reserve_exact(access_count)
            .map_err(|_| BoardExplorerError::Allocation)?;
        graph_accesses.extend(
            graph_resources
                .iter()
                .filter(|access| access.resource == descriptor.id)
                .copied(),
        );
        resources.push(BoardExplorerResource {
            descriptor,
            aliases: resource_aliases,
            graph_accesses,
        });
    }

    let mut visuals = Vec::new();
    visuals
        .try_reserve_exact(capability.visual_count())
        .map_err(|_| BoardExplorerError::Allocation)?;
    for visual in capability.visuals() {
        let mut hotspots = Vec::new();
        hotspots
            .try_reserve_exact(visual.hotspot_count())
            .map_err(|_| BoardExplorerError::Allocation)?;
        for hotspot in visual.hotspots() {
            let mut polygon = Vec::new();
            polygon
                .try_reserve_exact(hotspot.point_count())
                .map_err(|_| BoardExplorerError::Allocation)?;
            polygon.extend(hotspot.points());
            hotspots.push(BoardExplorerHotspot {
                id: try_owned(hotspot.id())?,
                resource: hotspot.resource(),
                polygon,
            });
        }
        visuals.push(BoardExplorerVisual {
            id: try_owned(visual.id())?,
            asset_path: try_owned(visual.asset_path())?,
            media_type: try_owned(visual.media_type())?,
            pixel_width: visual.pixel_width(),
            pixel_height: visual.pixel_height(),
            asset_digest: visual.asset_digest(),
            license: try_owned(visual.license())?,
            attribution: try_owned(visual.attribution())?,
            hotspots,
        });
    }

    Ok(BoardExplorerSnapshot {
        identity: capability.identity(),
        board_id: try_owned(capability.board_id())?,
        revision: try_owned(capability.revision())?,
        chip: capability.chip(),
        application_cores: capability.application_cores(),
        qualification: capability.qualification(),
        armable: capability.armable(),
        flash_bytes: capability.flash_bytes(),
        internal_sram_bytes: capability.internal_sram_bytes(),
        psram_bytes: capability.psram_bytes(),
        service_core: capability.service_core(),
        realtime_core: capability.realtime_core(),
        resources,
        alias_count: capability.alias_count(),
        graph_resource_count: capability.graph().resource_count(),
        bus_count: capability.bus_count(),
        device_count: capability.device_count(),
        flash_region_count: capability.flash_region_count(),
        clock_count: capability.clock_count(),
        electrical_constraint_count: capability.electrical_constraint_count(),
        interrupt_count: capability.interrupt_count(),
        safe_output_image_count: capability.safe_output_image_count(),
        visuals,
        hil_requirement_count: capability.hil_requirement_count(),
    })
}

fn try_owned(value: &str) -> Result<String, BoardExplorerError> {
    let mut owned = String::new();
    owned
        .try_reserve_exact(value.len())
        .map_err(|_| BoardExplorerError::Allocation)?;
    owned.push_str(value);
    Ok(owned)
}

#[cfg(test)]
mod tests {
    use alumina_board::{GraphResourceAccess, SafeValue};
    use alumina_capability::{MAX_CAPABILITY_CHUNK_BYTES, calculate_identity, read_verified_range};

    use super::*;

    fn tinybee_document() -> (CapabilityIdentity, Vec<u8>) {
        let package = &board_mks_tinybee::PACKAGE;
        let identity = calculate_identity(package).unwrap();
        let mut document = vec![0_u8; usize::try_from(identity.byte_len).unwrap()];
        let mut offset = 0_u32;
        while offset < identity.byte_len {
            let mut chunk = [0_u8; MAX_CAPABILITY_CHUNK_BYTES];
            let read = read_verified_range(package, offset, &mut chunk).unwrap();
            let start = usize::try_from(offset).unwrap();
            let count = usize::from(read.byte_len);
            document[start..start + count].copy_from_slice(&chunk[..count]);
            offset += u32::from(read.byte_len);
        }
        (identity, document)
    }

    #[test]
    fn tinybee_snapshot_separates_description_hazard_and_graph_access() {
        let (identity, document) = tinybee_document();
        let snapshot = build_board_explorer_snapshot(
            &document,
            identity,
            BoardCapabilityLimits::interactive(),
        )
        .unwrap();
        assert_eq!(snapshot.board_id(), board_mks_tinybee::PACKAGE.board.id);
        assert_eq!(snapshot.identity(), identity);
        assert_eq!(
            snapshot.resources().len(),
            board_mks_tinybee::RESOURCES.len()
        );
        assert_eq!(snapshot.alias_count(), board_mks_tinybee::ALIASES.len());
        assert_eq!(snapshot.graph_resource_count(), 4);
        assert!(snapshot.visuals().is_empty());
        assert!(!snapshot.armable());
        let summary = snapshot.resource_summary();
        assert_eq!(
            summary,
            BoardExplorerResourceSummary {
                service: 21,
                realtime: 41,
                hazardous: 21,
                graph_addressable: 4,
            }
        );
        assert_eq!(snapshot.supporting_section_counts(), [3, 1, 0, 2, 9, 4, 1]);
        assert_eq!(snapshot.hil_requirement_count(), 8);

        let limit_x = snapshot.resource(ResourceId::Gpio(33)).unwrap();
        assert_eq!(limit_x.descriptor().safe_value, SafeValue::HighImpedance);
        assert!(
            limit_x
                .aliases()
                .iter()
                .any(|alias| alias == "limit.x.negative")
        );
        assert_eq!(limit_x.graph_accesses().len(), 1);
        assert_eq!(
            limit_x.graph_accesses()[0].access,
            GraphResourceAccess::StableBooleanInput
        );

        let x_step = snapshot
            .resource(ResourceId::I2sOut {
                engine: 0,
                bit: board_mks_tinybee::X_STEP_BIT,
            })
            .unwrap();
        assert!(x_step.descriptor().hazardous_output);
        assert!(x_step.aliases().iter().any(|alias| alias == "axis.x.step"));
        assert!(!x_step.is_graph_addressable());
    }

    #[test]
    fn expected_identity_mismatch_fails_before_ui_state_is_returned() {
        let (identity, document) = tinybee_document();
        let wrong = CapabilityIdentity {
            byte_len: identity.byte_len,
            digest: Digest([0x44; 32]),
        };
        assert!(matches!(
            build_board_explorer_snapshot(
                &document,
                wrong,
                BoardCapabilityLimits::interactive()
            ),
            Err(BoardExplorerError::IdentityMismatch {
                expected,
                received
            }) if expected == wrong && received == identity
        ));
    }
}
