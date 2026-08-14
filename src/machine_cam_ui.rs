//! Offline, machine-bound exact CAM inspection for the canonical `TinyBee` fixture.
//!
//! The state below owns firmware-schema `ALMCFG06` bytes and artifacts derived
//! from them. The egui layer only projects retained exact values for display;
//! neither painter coordinates nor formatted text can flow back into CAM.

use alumina_board::{OwnerDomain, ResourceId};
use alumina_config::{
    BindingFlags, BindingRole, CONFIGURATION_HEADER_BYTES, CONFIGURATION_RECORD_BYTES,
    ConfigurationDocumentView, ConfigurationFlags, ConfigurationHeader, ConfigurationIdentity,
    ConfigurationRecord, ExactScalar, FactEvidence, MAX_CONFIGURATION_RECORDS,
    Rational as ConfigurationRational, ResourceBinding, ScalarFact, SignalPolarity,
};
use alumina_interface_core::{
    CanonicalDirectFiniteDifferenceProgram2, CanonicalDirectMotionEvidence1,
    CanonicalMachinePartition2, CanonicalScheduleEvidence3, CanonicalScheduledProgram2,
    CanonicalSharedScheduledGlobalJob2, CertifiedJerkSchedule2, CncGeometryImportLimits,
    CncGeometryImportReport2, DirectFiniteDifferencePolicy2, ExactValue, MachineDynamicsProfile2,
    MachinePartitionPolicy2, MachineResolutionBudget2, MetricPathApproximationLimits2, Millimetres,
    ScheduledLoweringLimits, SharedGlobalJobCompilePolicy2, SharedScheduledJobParticipant2,
    TimerDilationPolicy, build_canonical_schedule_evidence, build_direct_motion_evidence,
    certify_jerk_schedule, compile_shared_scheduled_global_job, import_exact_cnc_geometry,
    lower_certified_schedule_to_direct_finite_difference, lower_certified_schedule_to_v1,
    package_canonical_direct_program, package_canonical_scheduled_program, project_for_display,
    replay_canonical_schedule_evidence, replay_direct_motion_evidence, representative_curve_path,
    verify_canonical_schedule_evidence_bytes, verify_direct_motion_evidence_bytes,
};
use alumina_job::{JobNetworkPolicy, MachineJobGlobalFacts};
use alumina_machine_ir::{BlockValidationLimits, ValidationLimits};
use alumina_protocol::{DeviceId, Digest};
use alumina_sim::motion::{
    CachedFiniteDifferenceReplayReport, CachedStepperReplayReport,
    replay_cached_finite_difference_partition, replay_cached_stepper_partition,
};
use alumina_storage::{CacheLimits, UploadId, sha256};
use eframe::egui;
use hyperreal::{Rational, Real};

use crate::workspace_file::{BoundedFileBridge, BoundedFileEvent, BoundedFileSpec};

const MAXIMUM_CONFIGURATION_BYTES: usize =
    CONFIGURATION_HEADER_BYTES + MAX_CONFIGURATION_RECORDS * CONFIGURATION_RECORD_BYTES;
const MAXIMUM_EVIDENCE_BYTES: usize = 64 * 1024;
const MAXIMUM_CNC_SOURCE_BYTES: usize = CncGeometryImportLimits::INTERACTIVE.maximum_source_bytes();
const CONFIGURATION_FILE: BoundedFileSpec = BoundedFileSpec::new("ALMCFG06 file", "almcfg");
const EVIDENCE_FILE: BoundedFileSpec = BoundedFileSpec::new("ALMEVD03 file", "almevd");
const DIRECT_EVIDENCE_FILE: BoundedFileSpec = BoundedFileSpec::new("ALMDFE01 file", "almdfe");
const CNC_SOURCE_FILE: BoundedFileSpec = BoundedFileSpec::new("UI-only CNC geometry draft", "nc");
const STREAM_ID: [u8; 16] = *b"tinybee-cam-v1!!";
const DIRECT_STREAM_ID: [u8; 16] = *b"tinybee-direct01";
const UPLOAD_ID: UploadId = UploadId(0x1122_3344_5566_7788);
const DIRECT_UPLOAD_ID: UploadId = UploadId(0x1122_3344_5566_7799);
const SHARED_STREAM_1: [u8; 16] = *b"shared-mcu-0001!";
const SHARED_STREAM_2: [u8; 16] = *b"shared-mcu-0002!";
const SHARED_DEVICE_1: DeviceId = DeviceId([0x11; 16]);
const SHARED_DEVICE_2: DeviceId = DeviceId([0x22; 16]);
const PREPARE_ID: u64 = 0x8877_6655_4433_2211;
const MAXIMUM_VISIBLE_CNC_SPANS: usize = 128;
const CNC_EXAMPLE: &str = "%\n\
(selected exact geometry subset; feed/tool/process words are rejected)\n\
G21 G90 G17 G91.1\n\
N10 G0 X0 Y0\n\
N20 G1 X4 Y0\n\
N30 G2 X8 Y0 I2 J0\n\
M30\n\
%\n";

#[derive(Clone, Debug)]
enum MachineCamSource {
    ExactFixture {
        path: hypercurve::CurvePath2,
    },
    ImportedCnc {
        path: hypercurve::CurvePath2,
        raw_bytes: Vec<u8>,
        report: CncGeometryImportReport2,
    },
}

impl MachineCamSource {
    fn exact_fixture() -> Result<Self, String> {
        Ok(Self::ExactFixture {
            path: representative_curve_path()
                .map_err(|error| format!("retained exact source construction failed: {error}"))?,
        })
    }

    const fn path(&self) -> &hypercurve::CurvePath2 {
        match self {
            Self::ExactFixture { path } | Self::ImportedCnc { path, .. } => path,
        }
    }

    #[cfg(test)]
    const fn cnc_report(&self) -> Option<&CncGeometryImportReport2> {
        match self {
            Self::ExactFixture { .. } => None,
            Self::ImportedCnc { report, .. } => Some(report),
        }
    }

    fn cnc_parts(&self) -> Option<(&CncGeometryImportReport2, &[u8])> {
        match self {
            Self::ExactFixture { .. } => None,
            Self::ImportedCnc {
                raw_bytes, report, ..
            } => Some((report, raw_bytes)),
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::ExactFixture { .. } => "built-in exact Hypercurve fixture",
            Self::ImportedCnc { .. } => "selected UI-only CNC geometry import",
        }
    }
}

struct DirectMachineCamArtifacts {
    program: CanonicalDirectFiniteDifferenceProgram2,
    partition: CanonicalMachinePartition2,
    replay: CachedFiniteDifferenceReplayReport<2>,
    evidence: CanonicalDirectMotionEvidence1,
}

enum DirectMachineCamState {
    Ready(Box<DirectMachineCamArtifacts>),
    Unsupported(String),
}

struct MachineCamArtifacts {
    configuration_bytes: Vec<u8>,
    configuration_identity: ConfigurationIdentity,
    source: MachineCamSource,
    profile: MachineDynamicsProfile2,
    resolution_budget: MachineResolutionBudget2,
    schedule: CertifiedJerkSchedule2,
    program: CanonicalScheduledProgram2,
    partition: CanonicalMachinePartition2,
    replay: CachedStepperReplayReport<2>,
    evidence: CanonicalScheduleEvidence3,
    shared_job: CanonicalSharedScheduledGlobalJob2,
    direct: DirectMachineCamState,
}

impl MachineCamArtifacts {
    fn compile(configuration_bytes: Vec<u8>, source: MachineCamSource) -> Result<Self, String> {
        let configuration_digest = sha256(&configuration_bytes).digest;
        let view = ConfigurationDocumentView::decode::<MAX_CONFIGURATION_RECORDS>(
            &board_mks_tinybee::PACKAGE,
            &configuration_bytes,
            configuration_digest,
        )
        .map_err(|error| format!("ALMCFG06 validation rejected: {error:?}"))?;
        let configuration_identity = view.identity();
        let profile = MachineDynamicsProfile2::from_configuration(view)
            .map_err(|error| format!("machine profile derivation rejected: {error}"))?;
        let resolution_budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).map_err(|error| error.to_string())?,
            Rational::fraction(1, 100).map_err(|error| error.to_string())?,
            Rational::fraction(1, 100).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("machine resolution budget rejected: {error}"))?;
        let schedule = certify_jerk_schedule(
            source.path(),
            &profile,
            &resolution_budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .map_err(|error| format!("exact jerk scheduling rejected: {error}"))?;
        let program = lower_certified_schedule_to_v1(
            &schedule,
            &profile,
            &resolution_budget,
            Rational::fraction(1, 1_000).map_err(|error| error.to_string())?,
            ScheduledLoweringLimits::INTERACTIVE,
        )
        .map_err(|error| format!("canonical V1 lowering rejected: {error}"))?;
        let policy = machine_cam_partition_policy(STREAM_ID, UPLOAD_ID, &profile)?;
        let partition = package_canonical_scheduled_program(&program, policy)
            .map_err(|error| format!("cached partition packaging rejected: {error}"))?;
        let descriptor = partition
            .job_descriptor(PREPARE_ID)
            .map_err(|error| format!("cached job descriptor rejected: {error}"))?;
        let replay = replay_cached_stepper_partition::<2>(
            partition.bytes(),
            descriptor,
            profile.stepper_timing(0),
        )
        .map_err(|error| format!("deterministic cached replay rejected: {error:?}"))?;
        let evidence = build_canonical_schedule_evidence(&schedule, &program, &partition)
            .map_err(|error| format!("canonical evidence construction rejected: {error}"))?;
        replay_canonical_schedule_evidence(&evidence, &schedule, &program, &partition)
            .map_err(|error| format!("canonical evidence replay rejected: {error}"))?;

        let shared_job = compile_shared_machine_cam_job(&profile, &schedule, &program, &evidence)?;
        let direct = match compile_direct_machine_cam(&profile, &resolution_budget, &schedule) {
            Ok(artifacts) => DirectMachineCamState::Ready(Box::new(artifacts)),
            Err(error) => DirectMachineCamState::Unsupported(error),
        };

        Ok(Self {
            configuration_bytes,
            configuration_identity,
            source,
            profile,
            resolution_budget,
            schedule,
            program,
            partition,
            replay,
            evidence,
            shared_job,
            direct,
        })
    }
}

fn compile_direct_machine_cam(
    profile: &MachineDynamicsProfile2,
    resolution_budget: &MachineResolutionBudget2,
    schedule: &CertifiedJerkSchedule2,
) -> Result<DirectMachineCamArtifacts, String> {
    let policy = DirectFiniteDifferencePolicy2::interactive(
        Rational::fraction(1, 1_000).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("direct lowering policy rejected: {error}"))?;
    let program = lower_certified_schedule_to_direct_finite_difference(
        schedule,
        profile,
        resolution_budget,
        policy,
    )
    .map_err(|error| format!("direct finite-difference lowering unavailable: {error}"))?;
    let partition = package_canonical_direct_program(
        &program,
        machine_cam_partition_policy(DIRECT_STREAM_ID, DIRECT_UPLOAD_ID, profile)?,
    )
    .map_err(|error| format!("direct cached partition rejected: {error}"))?;
    let descriptor = partition
        .job_descriptor(PREPARE_ID)
        .map_err(|error| format!("direct job descriptor rejected: {error}"))?;
    let replay = replay_cached_finite_difference_partition::<2>(
        partition.bytes(),
        descriptor,
        profile.stepper_timing(0),
    )
    .map_err(|error| format!("direct cached replay rejected: {error:?}"))?;
    let evidence = build_direct_motion_evidence(schedule, &program, &partition)
        .map_err(|error| format!("direct evidence construction rejected: {error}"))?;
    replay_direct_motion_evidence(&evidence, schedule, &program, &partition)
        .map_err(|error| format!("direct evidence replay rejected: {error}"))?;
    Ok(DirectMachineCamArtifacts {
        program,
        partition,
        replay,
        evidence,
    })
}

fn compile_shared_machine_cam_job(
    profile: &MachineDynamicsProfile2,
    schedule: &CertifiedJerkSchedule2,
    program: &CanonicalScheduledProgram2,
    evidence: &CanonicalScheduleEvidence3,
) -> Result<CanonicalSharedScheduledGlobalJob2, String> {
    let global_template = MachineJobGlobalFacts {
        network_policy: JobNetworkPolicy::NetworkAttended,
        global_timebase_hz: 0,
        duration_ticks: 0,
        source_digest: evidence.source_digest(),
        compiler_digest: evidence.lowering_transcript_digest(),
        interface_digest: sha256(b"alumina-interface browser authority V1").digest,
        policy_digest: evidence.planner_transcript_digest(),
        machine_digest: profile.configuration_digest(),
        coordinate_epoch_digest: sha256(b"machine CAM fixture zero coordinate epoch V1").digest,
        safety_policy_digest: sha256(b"offline non-armable fixture safety policy V1").digest,
        synchronization_digest: Digest::ZERO,
    };
    let shared_policy = SharedGlobalJobCompilePolicy2::try_new(
        global_template,
        UploadId(0x3344_5566_7788_9900),
        700,
        CacheLimits {
            maximum_object_bytes: 4 * 1024 * 1024,
            maximum_chunk_bytes: 1_024,
            maximum_chunks: 10_000,
        },
    )
    .map_err(|error| format!("shared global policy rejected: {error}"))?;
    let board_package_digest = sha256(b"MKS TinyBee v1.0 board package").digest;
    let resource_set_digest = profile.configuration_digest();
    let safety_envelope_digest = global_template.safety_policy_digest;
    let first = SharedScheduledJobParticipant2::new(
        SHARED_DEVICE_1,
        board_package_digest,
        resource_set_digest,
        safety_envelope_digest,
        schedule,
        program,
        profile,
        machine_cam_partition_policy(SHARED_STREAM_1, UploadId(0x1122_3344_5566_0001), profile)?,
    );
    let second = SharedScheduledJobParticipant2::new(
        SHARED_DEVICE_2,
        board_package_digest,
        resource_set_digest,
        safety_envelope_digest,
        schedule,
        program,
        profile,
        machine_cam_partition_policy(SHARED_STREAM_2, UploadId(0x1122_3344_5566_0002), profile)?,
    );
    compile_shared_scheduled_global_job(
        shared_policy,
        TimerDilationPolicy::INTERACTIVE,
        vec![second, first],
    )
    .map_err(|error| format!("shared cached-job compilation rejected: {error}"))
}

fn machine_cam_partition_policy(
    stream_id: [u8; 16],
    upload_id: UploadId,
    profile: &MachineDynamicsProfile2,
) -> Result<MachinePartitionPolicy2, String> {
    MachinePartitionPolicy2::try_new(
        stream_id,
        profile.capability_digest(),
        profile.configuration_digest(),
        BlockValidationLimits {
            maximum_block_ticks: 10_000_000,
            segment: ValidationLimits {
                maximum_segment_ticks: 10_000_000,
                maximum_steps_per_segment: 100_000,
            },
        },
        upload_id,
        700,
        CacheLimits {
            maximum_object_bytes: 4 * 1024 * 1024,
            maximum_chunk_bytes: 1_024,
            maximum_chunks: 10_000,
        },
    )
    .map_err(|error| format!("partition policy rejected: {error}"))
}

/// Visible offline CAM state. It has no device-connection, arming, or output authority.
pub(crate) struct MachineCamWorkspace {
    artifacts: MachineCamArtifacts,
    selected_axis: usize,
    selected_point: usize,
    selected_segment: usize,
    configuration_file: BoundedFileBridge,
    evidence_file: BoundedFileBridge,
    direct_evidence_file: BoundedFileBridge,
    cnc_source_file: BoundedFileBridge,
    cnc_draft: String,
    file_status: String,
}

impl MachineCamWorkspace {
    pub(crate) fn try_new() -> Result<Self, String> {
        let source = MachineCamSource::exact_fixture()?;
        Ok(Self {
            artifacts: MachineCamArtifacts::compile(representative_configuration_bytes()?, source)?,
            selected_axis: 0,
            selected_point: 0,
            selected_segment: 0,
            configuration_file: BoundedFileBridge::default(),
            evidence_file: BoundedFileBridge::default(),
            direct_evidence_file: BoundedFileBridge::default(),
            cnc_source_file: BoundedFileBridge::default(),
            cnc_draft: CNC_EXAMPLE.to_owned(),
            file_status: "canonical offline fixture reconstructed; no file exchange this session"
                .to_owned(),
        })
    }

    pub(crate) fn show_sidebar(&self, ui: &mut egui::Ui) {
        let artifacts = &self.artifacts;
        ui.label("Authoritative machine-bound CAM");
        ui.strong(board_mks_tinybee::PACKAGE.board.id);
        ui.label(board_mks_tinybee::PACKAGE.board.revision);
        ui.monospace(format!(
            "config {}…",
            digest_prefix(artifacts.configuration_identity.digest.0)
        ));
        ui.monospace(format!(
            "capability {}…",
            digest_prefix(artifacts.configuration_identity.capability_digest.0)
        ));
        ui.separator();
        ui.label(artifacts.source.description());
        ui.label(format!(
            "{} source curves → {} metric elements · {} exact samples",
            artifacts.source.path().curves().len(),
            artifacts.schedule.route().len(),
            artifacts.program.points().len()
        ));
        ui.label(format!(
            "{} canonical segments · {} cache blocks",
            artifacts.program.segments().len(),
            artifacts.partition.block_count()
        ));
        match &artifacts.direct {
            DirectMachineCamState::Ready(direct) => ui.label(format!(
                "{} direct Q31.32 records · {} cache blocks · ALMDFE01 {}…",
                direct.program.records().len(),
                direct.partition.block_count(),
                digest_prefix(direct.evidence.digest().0)
            )),
            DirectMachineCamState::Unsupported(error) => {
                ui.weak(format!("direct Q31.32: typed blocker — {error}"))
            }
        };
        ui.label(format!(
            "{} synchronized MCU partitions · shared factor {} · ALMSYN01 {}…",
            artifacts.shared_job.global_job().participants().len(),
            artifacts.shared_job.retiming().selected_factor(),
            digest_prefix(artifacts.shared_job.timing_evidence().digest().0)
        ));
        ui.label(format!(
            "{} replayed rising edges",
            artifacts
                .replay
                .rising_edges
                .iter()
                .copied()
                .fold(0_u64, u64::saturating_add)
        ));
        ui.colored_label(
            egui::Color32::YELLOW,
            "Offline simulation only — no device, safety-chain, or arming evidence.",
        );
    }

    fn import_configuration_bytes(&mut self, bytes: Vec<u8>) -> Result<usize, String> {
        let byte_len = bytes.len();
        let artifacts = MachineCamArtifacts::compile(bytes, self.artifacts.source.clone())?;
        self.artifacts = artifacts;
        self.selected_point = 0;
        self.selected_segment = 0;
        Ok(byte_len)
    }

    fn import_cnc_source_bytes(&mut self, bytes: Vec<u8>) -> Result<usize, String> {
        let imported = import_exact_cnc_geometry(&bytes, CncGeometryImportLimits::INTERACTIVE)
            .map_err(|error| format!("selected CNC geometry import rejected: {error}"))?;
        let (path, report) = imported.into_parts();
        let byte_len = bytes.len();
        let source = MachineCamSource::ImportedCnc {
            path,
            raw_bytes: bytes,
            report,
        };
        let artifacts =
            MachineCamArtifacts::compile(self.artifacts.configuration_bytes.clone(), source)?;
        self.artifacts = artifacts;
        self.selected_point = 0;
        self.selected_segment = 0;
        Ok(byte_len)
    }

    fn restore_exact_fixture(&mut self) -> Result<(), String> {
        let source = MachineCamSource::exact_fixture()?;
        let artifacts =
            MachineCamArtifacts::compile(self.artifacts.configuration_bytes.clone(), source)?;
        self.artifacts = artifacts;
        self.selected_point = 0;
        self.selected_segment = 0;
        Ok(())
    }

    fn verify_evidence_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        verify_canonical_schedule_evidence_bytes(
            bytes,
            self.artifacts.evidence.digest(),
            &self.artifacts.schedule,
            &self.artifacts.program,
            &self.artifacts.partition,
        )
        .map_err(|error| error.to_string())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the exact machine, schedule, cache, and evidence views form one auditable workspace"
    )]
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.heading("Exact machine / CAM inspector");
            ui.colored_label(
                egui::Color32::LIGHT_BLUE,
                "ALMCFG06 → exact path → cached IR",
            );
        });
        ui.label(
            "The browser/WASM compiler is authoritative. Firmware-compatible configuration bytes determine every physical and electrical bound; diagnostic pixels below are one-way projections only.",
        );
        ui.colored_label(
            egui::Color32::YELLOW,
            "The TinyBee fixture is intentionally non-armable. This workspace initiates no device connection and cannot drive hardware.",
        );

        self.show_file_exchange(ui);
        ui.separator();
        self.show_cnc_source(ui);
        ui.separator();
        self.show_configuration(ui);
        ui.separator();
        self.show_exact_path_plot(ui);
        ui.separator();
        self.show_schedule(ui);
        ui.separator();
        self.show_lowering(ui);
        ui.separator();
        self.show_cache_replay(ui);
    }

    fn show_file_exchange(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Canonical artifacts", |ui| {
            let configuration_name = format!(
                "tinybee-{}.almcfg",
                digest_prefix(self.artifacts.configuration_identity.digest.0)
            );
            let events = self.configuration_file.show(
                ui,
                &self.artifacts.configuration_bytes,
                MAXIMUM_CONFIGURATION_BYTES,
                &configuration_name,
                CONFIGURATION_FILE,
            );
            for event in events {
                match event {
                    BoundedFileEvent::Import(Ok(bytes)) => {
                        match self.import_configuration_bytes(bytes) {
                            Ok(byte_len) => {
                                self.file_status = format!(
                                    "imported and transactionally replayed {byte_len} canonical ALMCFG06 bytes"
                                );
                            }
                            Err(error) => {
                                self.file_status = format!(
                                    "ALMCFG06 import rejected without changing the workspace: {error}"
                                );
                            }
                        }
                    }
                    BoundedFileEvent::Import(Err(error)) => {
                        self.file_status = format!("ALMCFG06 read rejected: {error}");
                    }
                    BoundedFileEvent::Export(Ok(bytes)) => {
                        self.file_status =
                            format!("exported {bytes} exact canonical ALMCFG06 bytes");
                    }
                    BoundedFileEvent::Export(Err(error)) => {
                        self.file_status = format!("ALMCFG06 export failed: {error}");
                    }
                }
            }

            let evidence_name = format!(
                "schedule-{}.almevd",
                digest_prefix(self.artifacts.evidence.digest().0)
            );
            let events = self.evidence_file.show(
                ui,
                self.artifacts.evidence.encoded(),
                MAXIMUM_EVIDENCE_BYTES,
                &evidence_name,
                EVIDENCE_FILE,
            );
            for event in events {
                match event {
                    BoundedFileEvent::Import(Ok(bytes)) => {
                        match self.verify_evidence_bytes(&bytes) {
                            Ok(()) => {
                                self.file_status = format!(
                                    "verified {} imported ALMEVD03 bytes against the current exact reconstruction",
                                    bytes.len()
                                );
                            }
                            Err(error) => {
                                self.file_status = format!(
                                    "ALMEVD03 import rejected without changing the workspace: {error}"
                                );
                            }
                        }
                    }
                    BoundedFileEvent::Import(Err(error)) => {
                        self.file_status = format!("ALMEVD03 read rejected: {error}");
                    }
                    BoundedFileEvent::Export(Ok(bytes)) => {
                        self.file_status =
                            format!("exported {bytes} canonical ALMEVD03 evidence bytes");
                    }
                    BoundedFileEvent::Export(Err(error)) => {
                        self.file_status = format!("ALMEVD03 export failed: {error}");
                    }
                }
            }
            self.show_direct_evidence_exchange(ui);
            ui.weak(&self.file_status);
        });
    }

    fn show_direct_evidence_exchange(&mut self, ui: &mut egui::Ui) {
        let DirectMachineCamState::Ready(direct) = &self.artifacts.direct else {
            return;
        };
        let evidence_name = format!(
            "direct-{}.almdfe",
            digest_prefix(direct.evidence.digest().0)
        );
        let events = self.direct_evidence_file.show(
            ui,
            direct.evidence.encoded(),
            MAXIMUM_EVIDENCE_BYTES,
            &evidence_name,
            DIRECT_EVIDENCE_FILE,
        );
        for event in events {
            match event {
                BoundedFileEvent::Import(Ok(bytes)) => match verify_direct_motion_evidence_bytes(
                    &bytes,
                    direct.evidence.digest(),
                    &self.artifacts.schedule,
                    &direct.program,
                    &direct.partition,
                ) {
                    Ok(()) => {
                        self.file_status = format!(
                            "verified {} imported ALMDFE01 bytes against the current direct reconstruction",
                            bytes.len()
                        );
                    }
                    Err(error) => {
                        self.file_status = format!(
                            "ALMDFE01 import rejected without changing the workspace: {error}"
                        );
                    }
                },
                BoundedFileEvent::Import(Err(error)) => {
                    self.file_status = format!("ALMDFE01 read rejected: {error}");
                }
                BoundedFileEvent::Export(Ok(bytes)) => {
                    self.file_status =
                        format!("exported {bytes} canonical ALMDFE01 evidence bytes");
                }
                BoundedFileEvent::Export(Err(error)) => {
                    self.file_status = format!("ALMDFE01 export failed: {error}");
                }
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the source admission policy, transaction, and provenance are kept visibly adjacent"
    )]
    fn show_cnc_source(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Exact source geometry");
            ui.colored_label(
                egui::Color32::LIGHT_BLUE,
                self.artifacts.source.description(),
            );
        });
        ui.label(
            "Optional CNC text is a bounded UI-only geometry importer. Raw text, modal words, and comments are never canonical job bytes and are never sent to firmware.",
        );
        ui.collapsing("Selected CNC semantics and admission limits", |ui| {
            ui.label(
                "Accepted: explicit G20/G21 units, G90/G91 endpoints, G17 XY plane, G90.1/G91.1 I/J centres, initial absolute G0 X/Y, connected G1/G2/G3 geometry, and terminal M2/M30.",
            );
            ui.label(
                "Rejected: feed, Z, spindle, tool, compensation, canned cycles, R arcs, full circles, rapid moves after retained geometry, and every unrecognized word or code.",
            );
            let limits = CncGeometryImportLimits::INTERACTIVE;
            ui.monospace(format!(
                "{} bytes · {} bytes/line · {} lines · {} words · {} curves · {} chars/decimal",
                limits.maximum_source_bytes(),
                limits.maximum_line_bytes(),
                limits.maximum_lines(),
                limits.maximum_words(),
                limits.maximum_curves(),
                limits.maximum_decimal_characters()
            ));
        });

        let events = self.cnc_source_file.show(
            ui,
            self.cnc_draft.as_bytes(),
            MAXIMUM_CNC_SOURCE_BYTES,
            "alumina-selected-geometry.nc",
            CNC_SOURCE_FILE,
        );
        ui.weak(
            "File controls move the editable draft only; compiling or opening an accepted file is the only operation that can replace the active exact source.",
        );
        for event in events {
            match event {
                BoundedFileEvent::Import(Ok(bytes)) => match String::from_utf8(bytes.clone()) {
                    Ok(draft) => match self.import_cnc_source_bytes(bytes) {
                        Ok(byte_len) => {
                            self.cnc_draft = draft;
                            self.file_status = format!(
                                "imported {byte_len} non-canonical CNC source bytes and transactionally rebuilt exact CAM, cached IR, simulation, and evidence"
                            );
                        }
                        Err(error) => {
                            self.file_status = format!(
                                "CNC source import rejected without changing machine/CAM state: {error}"
                            );
                        }
                    },
                    Err(error) => {
                        self.file_status = format!(
                            "CNC source read rejected without changing state: source is not UTF-8: {error}"
                        );
                    }
                },
                BoundedFileEvent::Import(Err(error)) => {
                    self.file_status = format!("CNC source read rejected: {error}");
                }
                BoundedFileEvent::Export(Ok(bytes)) => {
                    self.file_status = format!("exported {bytes} non-canonical CNC draft bytes");
                }
                BoundedFileEvent::Export(Err(error)) => {
                    self.file_status = format!("CNC draft export failed: {error}");
                }
            }
        }

        ui.collapsing("Edit/paste bounded CNC geometry draft", |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.cnc_draft)
                    .code_editor()
                    .desired_rows(9)
                    .char_limit(MAXIMUM_CNC_SOURCE_BYTES),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("compile draft transactionally").clicked() {
                    let result = if self.cnc_draft.len() > MAXIMUM_CNC_SOURCE_BYTES {
                        Err(format!(
                            "CNC draft has {} UTF-8 bytes; policy permits {MAXIMUM_CNC_SOURCE_BYTES}",
                            self.cnc_draft.len()
                        ))
                    } else {
                        self.import_cnc_source_bytes(self.cnc_draft.as_bytes().to_vec())
                    };
                    match result {
                        Ok(byte_len) => {
                            self.file_status = format!(
                                "compiled {byte_len} draft bytes through exact geometry, motion, cache, simulation, and evidence replay"
                            );
                        }
                        Err(error) => {
                            self.file_status = format!(
                                "CNC draft rejected without changing machine/CAM state: {error}"
                            );
                        }
                    }
                }
                if ui.button("restore built-in exact fixture").clicked() {
                    match self.restore_exact_fixture() {
                        Ok(()) => {
                            "restored the built-in exact Hypercurve fixture transactionally"
                                .clone_into(&mut self.file_status);
                        }
                        Err(error) => {
                            self.file_status =
                                format!("exact fixture restoration failed without mutation: {error}");
                        }
                    }
                }
            });
        });

        if let Some((report, raw_bytes)) = self.artifacts.source.cnc_parts() {
            ui.monospace(format!(
                "raw source {}… · exact geometry {}… · {} bytes / {} lines / {} words · {} positioning blocks · M-end line {}",
                digest_prefix(report.raw_source_digest().0),
                digest_prefix(self.artifacts.evidence.source_digest().0),
                raw_bytes.len(),
                report.source_lines(),
                report.parsed_words(),
                report.positioning_blocks(),
                report.program_end_line()
            ));
            ui.monospace(format!(
                "retained path: {} exact curves · ({}, {}) → ({}, {}) mm",
                report.spans().len(),
                report.start_mm().x(),
                report.start_mm().y(),
                report.end_mm().x(),
                report.end_mm().y()
            ));
            ui.collapsing("Imported per-curve source provenance", |ui| {
                egui::Grid::new("cnc_source_provenance")
                    .num_columns(7)
                    .striped(true)
                    .show(ui, |ui| {
                        for heading in ["curve", "line", "N", "motion", "units", "X/Y mode", "I/J"]
                        {
                            ui.strong(heading);
                        }
                        ui.end_row();
                        for span in report.spans().iter().take(MAXIMUM_VISIBLE_CNC_SPANS) {
                            ui.monospace(span.curve_index().to_string());
                            ui.monospace(span.source_line().to_string());
                            ui.monospace(
                                span.block_number()
                                    .map_or_else(|| "—".to_owned(), |value| value.to_string()),
                            );
                            ui.monospace(format!("{:?}", span.motion()));
                            ui.monospace(format!("{:?}", span.units()));
                            ui.monospace(format!("{:?}", span.distance_mode()));
                            ui.monospace(
                                span.arc_center_mode()
                                    .map_or_else(|| "—".to_owned(), |mode| format!("{mode:?}")),
                            );
                            ui.end_row();
                        }
                    });
                if report.spans().len() > MAXIMUM_VISIBLE_CNC_SPANS {
                    ui.weak(format!(
                        "{} additional spans are retained but omitted from this bounded table",
                        report.spans().len() - MAXIMUM_VISIBLE_CNC_SPANS
                    ));
                }
            });
        } else {
            ui.monospace(format!(
                "exact geometry {}… · no legacy text source or modal provenance by design",
                digest_prefix(self.artifacts.evidence.source_digest().0)
            ));
        }
        ui.weak(&self.file_status);
    }

    #[allow(
        clippy::too_many_lines,
        reason = "all canonical configuration facts and their exact derived budget remain in one inspectable panel"
    )]
    fn show_configuration(&mut self, ui: &mut egui::Ui) {
        let artifacts = &self.artifacts;
        let identity = artifacts.configuration_identity;
        ui.horizontal_wrapped(|ui| {
            ui.heading("Canonical machine configuration");
            ui.monospace(format!(
                "{} bytes · {} records · {} realtime",
                identity.byte_len,
                identity.summary.record_count,
                identity.summary.realtime_record_count
            ));
        });
        ui.label(format!(
            "{} stepper axes · {} safety binding · timer {} Hz · output quantum {} cycles",
            identity.summary.stepper_axes,
            if identity.summary.safety_binding {
                "present in validated config"
            } else {
                "missing"
            },
            artifacts.profile.timer_ticks_per_second(),
            artifacts.profile.output_quantum_cycles()
        ));
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.selected_axis, 0, "X axis");
            ui.selectable_value(&mut self.selected_axis, 1, "Y axis");
        });
        let axis = &artifacts.profile.axes()[self.selected_axis];
        egui::Grid::new("machine_axis_exact_facts")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                exact_row(
                    ui,
                    "step resource",
                    format!("{:?}", axis.step_binding().resource),
                );
                exact_row(
                    ui,
                    "direction resource",
                    format!("{:?}", axis.direction_binding().resource),
                );
                exact_row(
                    ui,
                    "driver control",
                    format!(
                        "{:?} via {:?}",
                        axis.driver_control_action(),
                        axis.driver_control_binding().resource
                    ),
                );
                exact_row(
                    ui,
                    "full steps / turn",
                    interval_text(axis.full_steps_per_turn()),
                );
                exact_row(ui, "microsteps", interval_text(axis.microsteps()));
                exact_row(
                    ui,
                    "command density (steps/mm)",
                    interval_text(axis.command_density_steps_per_millimetre()),
                );
                exact_row(
                    ui,
                    "usable travel (mm)",
                    format!(
                        "{} .. {}",
                        axis.usable_position_minimum_metres() * Rational::from(1_000),
                        axis.usable_position_maximum_metres() * Rational::from(1_000)
                    ),
                );
                exact_row(
                    ui,
                    "step-rate velocity ceiling (mm/s)",
                    (axis.step_rate_velocity_limit_metres_per_second() * Rational::from(1_000))
                        .to_string(),
                );
                exact_row(
                    ui,
                    "effective velocity (mm/s)",
                    (axis.effective_velocity_limit_metres_per_second() * Rational::from(1_000))
                        .to_string(),
                );
                exact_row(
                    ui,
                    "effective acceleration (mm/s²)",
                    (axis.effective_acceleration_limit_metres_per_second_squared()
                        * Rational::from(1_000))
                    .to_string(),
                );
                exact_row(
                    ui,
                    "effective jerk (mm/s³)",
                    (axis.effective_jerk_limit_metres_per_second_cubed() * Rational::from(1_000))
                        .to_string(),
                );
                exact_row(
                    ui,
                    "maximum following error (mm)",
                    (axis.maximum_following_error_metres() * Rational::from(1_000)).to_string(),
                );
            });

        ui.collapsing("Exact machine-wide resolution budget", |ui| {
            let budget = &artifacts.resolution_budget;
            egui::Grid::new("machine_resolution_budget")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    real_row(ui, "requested total", budget.requested_total_error_mm());
                    real_row(
                        ui,
                        "source-curve allocation",
                        budget.source_curve_allocation_mm(),
                    );
                    real_row(
                        ui,
                        "controller interpolation allocation",
                        budget.controller_interpolation_allocation_mm(),
                    );
                    real_row(
                        ui,
                        "endpoint quantization",
                        budget.endpoint_quantization_error_mm(),
                    );
                    real_row(
                        ui,
                        "step-event tracking",
                        budget.step_event_tracking_error_mm(),
                    );
                    real_row(ui, "command lattice", budget.command_lattice_error_mm());
                    real_row(ui, "calibration", budget.calibration_error_mm());
                    real_row(ui, "following", budget.following_error_mm());
                    real_row(
                        ui,
                        "output-grid position",
                        budget.output_grid_position_error_mm(),
                    );
                    real_row(
                        ui,
                        "certified required total",
                        budget.required_total_error_mm(),
                    );
                });
            ui.weak("All values are exact millimetres; no displayed decimal is re-imported.");
        });
    }

    fn show_exact_path_plot(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Exact source / certified motion path");
            ui.label(self.artifacts.source.description());
        });
        ui.weak(
            "The source remains native Hypercurve geometry. The plotted samples are the separately certified metric path consumed by motion control.",
        );
        let points = self.artifacts.program.points();
        if points.is_empty() {
            ui.colored_label(egui::Color32::RED, "scheduled point set is empty");
            return;
        }
        self.selected_point = self.selected_point.min(points.len() - 1);
        let width = ui.available_width().max(320.0);
        let (response, painter) =
            ui.allocate_painter(egui::vec2(width, 320.0), egui::Sense::hover());
        painter.rect_filled(response.rect, 5.0, egui::Color32::from_rgb(19, 23, 31));
        let envelope = self.artifacts.schedule.travel_envelope();
        let x_min = display_midpoint(&envelope.source_minimum_mm()[0]);
        let x_max = display_midpoint(&envelope.source_maximum_mm()[0]);
        let y_min = display_midpoint(&envelope.source_minimum_mm()[1]);
        let y_max = display_midpoint(&envelope.source_maximum_mm()[1]);
        let Some(world) = x_min
            .zip(x_max)
            .zip(y_min.zip(y_max))
            .map(|((x_min, x_max), (y_min, y_max))| [x_min, x_max, y_min, y_max])
        else {
            painter.text(
                response.rect.center(),
                egui::Align2::CENTER_CENTER,
                "exact path has no finite display projection",
                egui::FontId::proportional(14.0),
                egui::Color32::LIGHT_RED,
            );
            return;
        };
        let plot = response.rect.shrink(28.0);
        painter.rect_stroke(
            plot,
            0.0,
            egui::Stroke::new(1.0_f32, egui::Color32::DARK_GRAY),
        );
        for pair in points.windows(2) {
            let Some(start) = projected_point(pair[0].exact_point_mm(), plot, world) else {
                continue;
            };
            let Some(end) = projected_point(pair[1].exact_point_mm(), plot, world) else {
                continue;
            };
            let color = source_curve_color(pair[1].source_element());
            painter.line_segment([start, end], egui::Stroke::new(2.0_f32, color));
        }
        if let Some(selected) =
            projected_point(points[self.selected_point].exact_point_mm(), plot, world)
        {
            painter.circle_filled(selected, 5.0, egui::Color32::WHITE);
        }
        painter.text(
            plot.left_top(),
            egui::Align2::LEFT_TOP,
            "diagnostic projection only",
            egui::FontId::monospace(11.0),
            egui::Color32::GRAY,
        );
        ui.add(egui::Slider::new(&mut self.selected_point, 0..=points.len() - 1).text("sample"));
        let point = &points[self.selected_point];
        ui.monospace(format!(
            "source {} · motion {} · phase {} · subdivision {} · exact metric ({}, {}) mm",
            point.source_element(),
            point.motion_element(),
            point.phase_index(),
            point.subdivision_index(),
            point.exact_point_mm().x(),
            point.exact_point_mm().y()
        ));
        ui.monospace(format!(
            "ideal time {} s · tick {} · steps {:?}",
            point.ideal_time_seconds(),
            point.tick().get(),
            point
                .steps()
                .map(alumina_interface_core::CanonicalStep::get)
        ));
    }

    fn show_schedule(&self, ui: &mut egui::Ui) {
        let schedule = &self.artifacts.schedule;
        ui.heading("Hyperpath / Hypersolve schedule certificate");
        egui::Grid::new("exact_schedule_summary")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                real_row(
                    ui,
                    "certified metric path length (mm)",
                    schedule.total_path_length_mm(),
                );
                real_row(
                    ui,
                    "certified traversal time (s)",
                    schedule.total_traversal_time_seconds(),
                );
                real_row(
                    ui,
                    "maximum scalar feed (mm/s)",
                    schedule.limits().maximum_feed_mm_per_second(),
                );
                real_row(
                    ui,
                    "maximum scalar acceleration (mm/s²)",
                    schedule
                        .limits()
                        .maximum_acceleration_mm_per_second_squared(),
                );
                real_row(
                    ui,
                    "maximum scalar jerk (mm/s³)",
                    schedule.limits().maximum_jerk_mm_per_second_cubed(),
                );
            });
        self.show_axis_projection(ui);
        ui.monospace(format!(
            "{} retained source curves → {} certified metric elements · maximum source-to-motion error {} mm",
            schedule.source().curves().len(),
            schedule.metric_path().path().curves().len(),
            schedule.metric_path().maximum_source_error_mm_exact()
        ));
        self.show_source_to_motion_certificate(ui);
        let envelope = schedule.travel_envelope();
        ui.monospace(format!(
            "source envelope X {}..{} mm · Y {}..{} mm",
            envelope.source_minimum_mm()[0],
            envelope.source_maximum_mm()[0],
            envelope.source_minimum_mm()[1],
            envelope.source_maximum_mm()[1]
        ));
        ui.monospace(format!(
            "usable travel X {}..{} mm · Y {}..{} mm · certified inside",
            envelope.usable_minimum_mm()[0],
            envelope.usable_maximum_mm()[0],
            envelope.usable_minimum_mm()[1],
            envelope.usable_maximum_mm()[1]
        ));
        let maximum_jerk_refinement = schedule
            .lookahead_plan()
            .positive_node_components
            .iter()
            .map(|component| component.uniform_halvings)
            .max()
            .unwrap_or(0);
        ui.label(format!(
            "exact acceleration + jerk-feasible lookahead: {} nodes / {} joins / {} spans, {} positive components, at most {} exact halvings · jerk replay: {} elements, all certified",
            schedule
                .acceleration_lookahead_plan()
                .effective_node_feed_limits
                .len(),
            schedule.lookahead().corner_feeds.len(),
            schedule.lookahead_report().spans.len(),
            schedule.lookahead_plan().positive_node_components.len(),
            maximum_jerk_refinement,
            schedule.jerk_report().elements.len()
        ));
        ui.label(
            "active phase policy: positive feed only across lossless exact line-to-line G1 continuations; curvature-bearing joins, corners, reversals, and certified cubic chords remain full stops",
        );
        self.show_jerk_phases(ui);
    }

    fn show_axis_projection(&self, ui: &mut egui::Ui) {
        let Some(projection) = self.artifacts.schedule.limits().affine_axis_projection() else {
            ui.label(
                "axis projection: conservative direction-independent limits retained because the metric route contains a curved span; affine |dq/ds| limits are not reused across curvature",
            );
            return;
        };
        ui.label(format!(
            "exact affine dense-axis projection: {} span/axis rows · feed bottleneck span {} axis {} · acceleration span {} axis {} · jerk span {} axis {} · all replayed",
            projection.certification.rows.len(),
            projection.feed_bottleneck.span_index,
            projection.feed_bottleneck.axis_index,
            projection.acceleration_bottleneck.span_index,
            projection.acceleration_bottleneck.axis_index,
            projection.jerk_bottleneck.span_index,
            projection.jerk_bottleneck.axis_index,
        ));
        ui.collapsing("Affine axis-projection certificate", |ui| {
            egui::Grid::new("affine_axis_projection_rows")
                .num_columns(7)
                .striped(true)
                .show(ui, |ui| {
                    for heading in [
                        "span", "axis", "|dq/ds|", "axis V", "axis A", "axis J", "replay",
                    ] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for row in &projection.certification.rows {
                        ui.monospace(row.span_index.to_string());
                        ui.monospace(row.axis_index.to_string());
                        ui.monospace(row.absolute_axis_derivative.to_string());
                        ui.monospace(row.axis_limits.maximum_velocity.to_string());
                        ui.monospace(row.axis_limits.maximum_acceleration.to_string());
                        ui.monospace(row.axis_limits.maximum_jerk.to_string());
                        ui.monospace(if row.certification.all_satisfied() {
                            "certified"
                        } else {
                            "rejected"
                        });
                        ui.end_row();
                    }
                });
        });
    }

    fn show_source_to_motion_certificate(&self, ui: &mut egui::Ui) {
        let metric_path = self.artifacts.schedule.metric_path();
        ui.collapsing("Source-to-motion certificate", |ui| {
            egui::Grid::new("source_to_motion_spans")
                .num_columns(6)
                .striped(true)
                .show(ui, |ui| {
                    for heading in [
                        "source",
                        "family",
                        "motion range",
                        "elements",
                        "error mm",
                        "depth",
                    ] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for span in metric_path.spans() {
                        let motion_end = span
                            .motion_element_start()
                            .saturating_add(span.motion_element_count());
                        ui.monospace(span.source_element().to_string());
                        ui.monospace(format!("{:?}", span.source_family()));
                        ui.monospace(format!(
                            "{}..{}",
                            span.motion_element_start(),
                            motion_end
                        ));
                        ui.monospace(span.motion_element_count().to_string());
                        ui.monospace(span.maximum_error_mm_exact().to_string());
                        ui.monospace(span.maximum_subdivision_depth().to_string());
                        ui.end_row();
                    }
                });
            ui.weak(
                "Every curvature-bearing join, true corner, reversal, and certified cubic chord boundary is an exact zero-feed node; only lossless exact line-to-line G1 continuations may remain moving.",
            );
        });
    }

    fn show_jerk_phases(&self, ui: &mut egui::Ui) {
        let schedule = &self.artifacts.schedule;
        for (element, phases) in schedule.phases().iter().enumerate() {
            let source_element = schedule
                .metric_path()
                .source_element_for_motion(element)
                .map_or_else(|| "?".to_owned(), |source| source.to_string());
            ui.collapsing(
                format!(
                    "motion {element} / source {source_element} · {} constant-jerk phases",
                    phases.len()
                ),
                |ui| {
                    egui::Grid::new(("jerk_phases", element))
                        .num_columns(7)
                        .striped(true)
                        .show(ui, |ui| {
                            for heading in ["phase", "length", "time", "v₀", "v₁", "a₀", "a₁"]
                            {
                                ui.strong(heading);
                            }
                            ui.end_row();
                            for (phase, proposal) in phases.iter().enumerate() {
                                ui.monospace(phase.to_string());
                                ui.monospace(proposal.path_length.to_string());
                                ui.monospace(proposal.ramp.traversal_time.to_string());
                                ui.monospace(proposal.ramp.start_feed.to_string());
                                ui.monospace(proposal.ramp.end_feed.to_string());
                                ui.monospace(proposal.ramp.start_acceleration.to_string());
                                ui.monospace(proposal.ramp.end_acceleration.to_string());
                                ui.end_row();
                            }
                        });
                },
            );
        }
    }

    fn show_lowering(&mut self, ui: &mut egui::Ui) {
        let program = &self.artifacts.program;
        ui.heading("Canonical V1 lowering and executor preflight");
        let evidence = program.evidence();
        ui.monospace(format!(
            "retained point budget: {} / {}",
            program.points().len(),
            ScheduledLoweringLimits::INTERACTIVE.maximum_points()
        ));
        let timer_lattice = evidence.timer_lattice_schedule();
        egui::Grid::new("lowering_evidence")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                real_row(
                    ui,
                    "maximum source-to-motion (mm)",
                    evidence.maximum_source_to_motion_error_mm(),
                );
                real_row(
                    ui,
                    "requested interpolation (mm)",
                    evidence.requested_interpolation_error_mm(),
                );
                real_row(
                    ui,
                    "maximum certified chord error (mm)",
                    evidence.maximum_chord_interpolation_error_mm(),
                );
                real_row(
                    ui,
                    "maximum endpoint quantization (mm)",
                    evidence.maximum_position_quantization_error_mm(),
                );
                real_row(
                    ui,
                    "maximum DDA tracking (mm)",
                    evidence.maximum_step_event_tracking_error_mm(),
                );
                real_row(
                    ui,
                    "curve-to-canonical bound (mm)",
                    evidence.maximum_curve_to_canonical_error_mm(),
                );
                real_row(
                    ui,
                    "maximum timer cumulative delay (s)",
                    timer_lattice.maximum_cumulative_delay_seconds(),
                );
                real_row(
                    ui,
                    "maximum segment extension (s)",
                    timer_lattice.maximum_segment_extension_seconds(),
                );
                real_row(
                    ui,
                    "maximum output-grid padding (s)",
                    timer_lattice.maximum_output_grid_padding_seconds(),
                );
            });
        ui.monospace(format!(
            "timer lattice: selected factor {} on 1/{} grid through {}/{} ({} complete preflight replays) · factor-one rejection {:?} · immediate predecessor {:?}",
            timer_lattice.selected_factor(),
            timer_lattice.selected_factor_denominator(),
            timer_lattice.maximum_factor_numerator(),
            timer_lattice.selected_factor_denominator(),
            timer_lattice.candidate_replays(),
            timer_lattice.unit_factor_rejection(),
            timer_lattice.predecessor_rejection()
        ));
        let preflight = program.executor_preflight();
        ui.monospace(format!(
            "preflight: {} segments · end tick {} · terminal {:?} · rising edges {:?} · disable legal at cycle {}",
            preflight.segment_count,
            preflight.end_tick.0,
            preflight.position,
            preflight.emitted_steps,
            preflight.earliest_finish_cycle.0
        ));
        if !program.segments().is_empty() {
            self.selected_segment = self.selected_segment.min(program.segments().len() - 1);
            ui.add(
                egui::Slider::new(&mut self.selected_segment, 0..=program.segments().len() - 1)
                    .text("canonical segment"),
            );
            let segment = program.segments()[self.selected_segment];
            ui.monospace(format!(
                "segment {}: ticks {}..{} · Δsteps {:?} · flags {}",
                self.selected_segment,
                segment.start_tick.0,
                segment.end_tick.0,
                segment.delta_steps,
                segment.flags
            ));
        }
    }

    fn show_cache_replay(&self, ui: &mut egui::Ui) {
        let partition = &self.artifacts.partition;
        let publication = partition.publication();
        let replay = self.artifacts.replay;
        ui.heading("SD-cache artifact and deterministic firmware replay");
        ui.monospace(format!(
            "partition {}… · {} bytes · manifest {}…",
            digest_prefix(publication.object.content.digest.0),
            publication.object.byte_len,
            digest_prefix(publication.manifest.digest.0)
        ));
        ui.label(format!(
            "{} blocks · {} independently hashed chunks · longest block {} ticks",
            partition.block_count(),
            partition.chunks().len(),
            partition.maximum_observed_block_ticks()
        ));
        ui.monospace(format!(
            "replay: {} blocks / {} segments · terminal tick {} · terminal position {:?} · rising edges {:?}",
            replay.block_count,
            replay.segment_count,
            replay.terminal_tick.0,
            replay.terminal_position,
            replay.rising_edges
        ));
        ui.monospace(format!(
            "{} output transactions · finish cycle {} · terminal block {}…",
            replay.output_transactions,
            replay.finish_cycle.0,
            digest_prefix(replay.terminal_block_digest.0)
        ));
        ui.monospace(format!(
            "evidence {}… · source {}… · metric {}… · approximation {}…",
            digest_prefix(self.artifacts.evidence.digest().0),
            digest_prefix(self.artifacts.evidence.source_digest().0),
            digest_prefix(self.artifacts.evidence.metric_path_digest().0),
            digest_prefix(self.artifacts.evidence.source_approximation_digest().0),
        ));
        ui.monospace(format!(
            "planner {}… / {} canonical bytes · lowering {}… / {} canonical bytes",
            digest_prefix(self.artifacts.evidence.planner_transcript_digest().0),
            self.artifacts.evidence.planner_transcript_byte_len(),
            digest_prefix(self.artifacts.evidence.lowering_transcript_digest().0),
            self.artifacts.evidence.lowering_transcript_byte_len(),
        ));
        ui.monospace(format!(
            "{} canonical ALMEVD03 bytes",
            self.artifacts.evidence.encoded().len()
        ));
        self.show_direct_cache_replay(ui);
        let shared = &self.artifacts.shared_job;
        let timing = shared.retiming();
        let shared_evidence = shared.timing_evidence();
        let global = shared.global_job();
        ui.separator();
        ui.strong("Joint multi-MCU retiming before immutable publication");
        ui.monospace(format!(
            "factor {} on 1/{} grid · {} complete rounds / {} participant replays · common terminal tick {}",
            timing.selected_factor(),
            timing.selected_factor_denominator(),
            timing.candidate_rounds(),
            timing.participant_replays(),
            timing.terminal_tick().get()
        ));
        ui.monospace(format!(
            "ALMSYN01 {}… / {} bytes · ALMSRT01 {}… / {} streamed canonical bytes",
            digest_prefix(shared_evidence.digest().0),
            shared_evidence.encoded().len(),
            digest_prefix(shared_evidence.transcript_digest().0),
            shared_evidence.transcript_byte_len()
        ));
        ui.monospace(format!(
            "global manifest {}… · {} participants · {} bytes / {} chunks",
            digest_prefix(global.global_job_digest().0),
            global.participants().len(),
            global.manifest_bytes().len(),
            global.manifest_chunks().len()
        ));
        for (participant, report) in global.participants().iter().zip(timing.participants()) {
            let id = report.device_id().0;
            ui.monospace(format!(
                "MCU {:02x}{:02x}{:02x}{:02x}… · {} blocks / {} bytes · unit {:?} · predecessor {:?}",
                id[0],
                id[1],
                id[2],
                id[3],
                participant.partition().block_count(),
                participant.partition().bytes().len(),
                report.unit_factor_outcome(),
                report.predecessor_outcome()
            ));
        }
        ui.colored_label(
            egui::Color32::LIGHT_BLUE,
            "Immutable bytes were decoded, admitted, event-replayed, acknowledged, and cross-checked through the production core-1 owners.",
        );
    }

    fn show_direct_cache_replay(&self, ui: &mut egui::Ui) {
        ui.separator();
        ui.strong("Direct Q31.32 finite-difference lowering");
        match &self.artifacts.direct {
            DirectMachineCamState::Ready(direct) => {
                let replay = direct.replay;
                ui.monospace(format!(
                    "{} records / {} dense updates → {} ALMBLK02 blocks · maximum {} updates/record",
                    direct.program.records().len(),
                    replay.update_count,
                    direct.partition.block_count(),
                    direct.partition.maximum_finite_difference_updates()
                ));
                ui.monospace(format!(
                    "terminal tick {} · integer {:?} · Q31.32 {:?} · rising edges {:?}",
                    replay.terminal_tick.0,
                    replay.terminal_position,
                    replay.terminal_finite_position,
                    replay.rising_edges
                ));
                ui.monospace(format!(
                    "coefficient error ≤ {} mm of {} mm allocation",
                    direct.program.evidence().maximum_position_error_mm(),
                    direct
                        .program
                        .evidence()
                        .policy()
                        .maximum_position_error_mm()
                ));
                ui.monospace(format!(
                    "ALMDFE01 {}… / {} bytes · ALMDFT01 {}… / {} streamed exact bytes",
                    digest_prefix(direct.evidence.digest().0),
                    direct.evidence.encoded().len(),
                    digest_prefix(direct.evidence.transcript_digest().0),
                    direct.evidence.transcript_byte_len()
                ));
            }
            DirectMachineCamState::Unsupported(error) => {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("Typed direct-lowering blocker: {error}"),
                );
                ui.weak(
                    "The canonical V1 preview above remains available; firmware receives no implicit compatibility fallback.",
                );
            }
        }
    }
}

fn wire_rational(numerator: i64, denominator: u64) -> Result<ConfigurationRational, String> {
    ConfigurationRational::new(numerator, denominator)
        .map_err(|error| format!("configuration rational rejected: {error:?}"))
}

fn binding(
    instance: u16,
    role: BindingRole,
    resource: ResourceId,
    polarity: SignalPolarity,
) -> ConfigurationRecord {
    let safety = role == BindingRole::EmergencyStop;
    ConfigurationRecord::Binding(ResourceBinding {
        instance,
        role,
        resource,
        owner: OwnerDomain::Realtime,
        polarity,
        flags: BindingFlags(if safety {
            BindingFlags::REQUIRED_INTERLOCK
        } else {
            0
        }),
        minimum_active_cycles: 48,
        minimum_inactive_cycles: 48,
        maximum_frequency_hz: if safety { 0 } else { 100_000 },
        watchdog_cycles: 240_000,
    })
}

fn scalar(
    instance: u16,
    fact: ScalarFact,
    numerator: i64,
    denominator: u64,
    uncertainty_numerator: i64,
    uncertainty_denominator: u64,
) -> Result<ConfigurationRecord, String> {
    Ok(ConfigurationRecord::Scalar(ExactScalar {
        instance,
        fact,
        value: wire_rational(numerator, denominator)?,
        uncertainty: wire_rational(uncertainty_numerator, uncertainty_denominator)?,
        evidence: FactEvidence::Declared,
    }))
}

fn axis_scalars(instance: u16) -> Result<Vec<ConfigurationRecord>, String> {
    Ok(vec![
        scalar(instance, ScalarFact::AxisFullStepsPerTurn, 200, 1, 0, 1)?,
        scalar(instance, ScalarFact::AxisMicrosteps, 16, 1, 0, 1)?,
        scalar(
            instance,
            ScalarFact::AxisMotorTurnsPerOutputTurn,
            1,
            1,
            0,
            1,
        )?,
        scalar(
            instance,
            ScalarFact::AxisTravelMetresPerOutputTurn,
            1,
            500,
            0,
            1,
        )?,
        scalar(
            instance,
            ScalarFact::AxisCalibrationScale,
            1,
            1,
            1,
            1_000_000,
        )?,
        scalar(instance, ScalarFact::AxisPositionMinimumMetres, 0, 1, 0, 1)?,
        scalar(instance, ScalarFact::AxisPositionMaximumMetres, 3, 10, 0, 1)?,
        scalar(
            instance,
            ScalarFact::AxisVelocityLimitMetresPerSecond,
            1,
            20,
            1,
            1_000,
        )?,
        scalar(
            instance,
            ScalarFact::AxisAccelerationLimitMetresPerSecondSquared,
            1,
            2,
            1,
            100,
        )?,
        scalar(
            instance,
            ScalarFact::AxisJerkLimitMetresPerSecondCubed,
            5,
            1,
            1,
            10,
        )?,
        scalar(
            instance,
            ScalarFact::AxisFollowingErrorMetres,
            1,
            100_000,
            1,
            500_000,
        )?,
    ])
}

fn representative_configuration_bytes() -> Result<Vec<u8>, String> {
    let mut records = vec![
        binding(
            0,
            BindingRole::AxisStep,
            ResourceId::I2sOut { engine: 0, bit: 1 },
            SignalPolarity::ActiveHigh,
        ),
        binding(
            0,
            BindingRole::AxisDirection,
            ResourceId::I2sOut { engine: 0, bit: 2 },
            SignalPolarity::ActiveHigh,
        ),
        binding(
            0,
            BindingRole::AxisDisable,
            ResourceId::I2sOut { engine: 0, bit: 0 },
            SignalPolarity::ActiveHigh,
        ),
        binding(
            0,
            BindingRole::EmergencyStop,
            ResourceId::Gpio(33),
            SignalPolarity::ActiveLow,
        ),
        binding(
            1,
            BindingRole::AxisStep,
            ResourceId::I2sOut { engine: 0, bit: 4 },
            SignalPolarity::ActiveHigh,
        ),
        binding(
            1,
            BindingRole::AxisDirection,
            ResourceId::I2sOut { engine: 0, bit: 5 },
            SignalPolarity::ActiveHigh,
        ),
        binding(
            1,
            BindingRole::AxisDisable,
            ResourceId::I2sOut { engine: 0, bit: 3 },
            SignalPolarity::ActiveHigh,
        ),
    ];
    records.extend(axis_scalars(0)?);
    records.push(scalar(0, ScalarFact::TimerTickHertz, 1_000_000, 1, 0, 1)?);
    records.push(scalar(
        0,
        ScalarFact::StepperOutputQuantumCycles,
        1,
        1,
        0,
        1,
    )?);
    records.extend(axis_scalars(1)?);
    records.sort_by_key(|record| record.canonical_order_key());
    let realtime_record_count = records
        .iter()
        .filter(|record| record.realtime_relevant())
        .count();
    let header = ConfigurationHeader {
        capability_digest: board_mks_tinybee::CAPABILITY_DIGEST,
        record_count: u16::try_from(records.len())
            .map_err(|_| "configuration record count exceeds u16".to_owned())?,
        realtime_record_count: u16::try_from(realtime_record_count)
            .map_err(|_| "realtime configuration record count exceeds u16".to_owned())?,
        flags: ConfigurationFlags(
            ConfigurationFlags::MOTION | ConfigurationFlags::CACHED_AUTONOMOUS,
        ),
    };
    let mut bytes = Vec::from(
        header
            .encode()
            .map_err(|error| format!("configuration header rejected: {error:?}"))?,
    );
    for record in records {
        bytes.extend_from_slice(
            &record
                .encode()
                .map_err(|error| format!("configuration record rejected: {error:?}"))?,
        );
    }
    Ok(bytes)
}

fn exact_row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.label(label);
    ui.monospace(value);
    ui.end_row();
}

fn real_row(ui: &mut egui::Ui, label: &str, value: &Real) {
    exact_row(ui, label, value.to_string());
}

fn interval_text(interval: &alumina_interface_core::ExactInterval) -> String {
    format!(
        "{} [{} .. {}]",
        interval.nominal(),
        interval.lower(),
        interval.upper()
    )
}

fn digest_prefix(digest: [u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut result = String::with_capacity(16);
    for byte in &digest[..8] {
        write!(&mut result, "{byte:02x}").expect("writing into a string is infallible");
    }
    result
}

fn source_curve_color(source_element: usize) -> egui::Color32 {
    const COLORS: [egui::Color32; 4] = [
        egui::Color32::from_rgb(96, 169, 232),
        egui::Color32::from_rgb(241, 178, 84),
        egui::Color32::from_rgb(125, 211, 164),
        egui::Color32::from_rgb(205, 145, 232),
    ];
    COLORS[source_element % COLORS.len()]
}

fn display_midpoint(value: &Real) -> Option<f64> {
    project_for_display(&ExactValue::<Millimetres>::from_real(value.clone()))
        .ok()
        .map(alumina_interface_core::DisplayScalar::get)
}

fn projected_point(
    point: &hypercurve::Point2,
    rect: egui::Rect,
    world: [f64; 4],
) -> Option<egui::Pos2> {
    let x = display_midpoint(point.x())?;
    let y = display_midpoint(point.y())?;
    let width = (world[1] - world[0]).max(f64::EPSILON);
    let height = (world[3] - world[2]).max(f64::EPSILON);
    let normalized_x = ((x - world[0]) / width).clamp(0.0, 1.0);
    let normalized_y = ((y - world[2]) / height).clamp(0.0, 1.0);
    Some(egui::pos2(
        rect.left() + display_unit(normalized_x) * rect.width(),
        rect.bottom() - display_unit(normalized_y) * rect.height(),
    ))
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "audited one-way finite display projection into egui's f32 coordinate space"
)]
fn display_unit(value: f64) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_fixture_replays_from_configuration_through_cached_events() {
        let workspace = MachineCamWorkspace::try_new().unwrap();
        let artifacts = &workspace.artifacts;
        assert_eq!(&artifacts.configuration_bytes[..8], b"ALMCFG06");
        assert!(!board_mks_tinybee::PACKAGE.armable);
        assert_eq!(artifacts.configuration_identity.summary.stepper_axes, 2);
        assert!(artifacts.configuration_identity.summary.safety_binding);
        assert!(artifacts.schedule.lookahead_report().all_satisfied());
        assert!(artifacts.schedule.jerk_report().all_satisfied());
        assert_eq!(artifacts.schedule.source().curves().len(), 3);
        assert!(artifacts.schedule.metric_path().path().curves().len() > 3);
        assert_eq!(
            artifacts
                .schedule
                .metric_path()
                .maximum_source_error_mm_exact(),
            &Rational::fraction(1, 100).unwrap()
        );
        assert_eq!(
            artifacts.program.points().len(),
            artifacts.program.segments().len() + 1
        );
        assert_eq!(
            artifacts.replay.terminal_position,
            artifacts.partition.final_position()
        );
        assert_eq!(&artifacts.evidence.encoded()[..8], b"ALMEVD03");
        assert_ne!(
            artifacts.evidence.source_digest(),
            artifacts.evidence.metric_path_digest()
        );
        assert_eq!(
            artifacts.shared_job.retiming().selected_factor_numerator(),
            4_096
        );
        assert_eq!(artifacts.shared_job.retiming().candidate_rounds(), 1);
        assert_eq!(artifacts.shared_job.retiming().participant_replays(), 2);
        assert_eq!(artifacts.shared_job.global_job().participants().len(), 2);
        assert_eq!(
            &artifacts.shared_job.timing_evidence().encoded()[..8],
            b"ALMSYN01"
        );
        assert_eq!(artifacts.shared_job.timing_evidence().encoded().len(), 104);
        assert_eq!(
            artifacts
                .shared_job
                .global_job()
                .policy()
                .global()
                .synchronization_digest,
            artifacts.shared_job.timing_evidence().digest()
        );
        assert!(
            artifacts
                .shared_job
                .global_job()
                .participant_records()
                .iter()
                .all(|participant| participant.error_evidence_digest
                    == artifacts.shared_job.timing_evidence().digest())
        );
        assert!(matches!(
            &artifacts.direct,
            DirectMachineCamState::Unsupported(_)
        ));
    }

    #[test]
    fn retired_configuration_v5_is_rejected_without_a_compatibility_path() {
        let workspace = MachineCamWorkspace::try_new().unwrap();
        let mut legacy = workspace.artifacts.configuration_bytes.clone();
        legacy[..8].copy_from_slice(b"ALMCFG05");
        legacy[8..10].copy_from_slice(&5_u16.to_le_bytes());
        let result = MachineCamArtifacts::compile(legacy, workspace.artifacts.source.clone());
        let Err(error) = result else {
            panic!("retired configuration unexpectedly compiled");
        };
        assert!(error.starts_with("ALMCFG06 validation rejected:"));
    }

    #[test]
    fn affine_cnc_source_exposes_replayed_direct_ir_and_evidence() {
        let mut workspace = MachineCamWorkspace::try_new().unwrap();
        let source = b"G21 G90 G17 G91.1\nG0 X0 Y0\nG1 X1 Y0\nM2\n";
        workspace.import_cnc_source_bytes(source.to_vec()).unwrap();
        let DirectMachineCamState::Ready(direct) = &workspace.artifacts.direct else {
            panic!("one stop-to-stop affine span must support direct lowering")
        };
        assert!(!direct.program.records().is_empty());
        assert_eq!(direct.replay.terminal_position, [1_600, 0]);
        assert_eq!(
            direct.replay.update_count,
            direct.program.executor_preflight().update_count
        );
        assert_eq!(&direct.evidence.encoded()[..8], b"ALMDFE01");
        assert_eq!(
            direct.partition.terminal_finite_position(),
            Some(direct.replay.terminal_finite_position)
        );
    }

    #[test]
    fn invalid_configuration_is_rejected_before_visible_state_can_change() {
        let mut workspace = MachineCamWorkspace::try_new().unwrap();
        let retained_digest = workspace.artifacts.configuration_identity.digest;
        let retained_segments = workspace.artifacts.program.segments().len();
        let mut corrupt = workspace.artifacts.configuration_bytes.clone();
        corrupt[0] ^= 1;
        assert!(workspace.import_configuration_bytes(corrupt).is_err());
        assert_eq!(
            workspace.artifacts.configuration_identity.digest,
            retained_digest
        );
        assert_eq!(
            workspace.artifacts.program.segments().len(),
            retained_segments
        );
    }

    #[test]
    fn evidence_import_requires_the_current_reconstructed_identity() {
        let workspace = MachineCamWorkspace::try_new().unwrap();
        assert!(
            workspace
                .verify_evidence_bytes(workspace.artifacts.evidence.encoded())
                .is_ok()
        );
        let mut corrupt = workspace.artifacts.evidence.encoded().to_vec();
        corrupt[12] ^= 1;
        assert!(workspace.verify_evidence_bytes(&corrupt).is_err());
    }

    #[test]
    fn selected_cnc_text_reconstructs_geometry_without_becoming_canonical_identity() {
        let mut workspace = MachineCamWorkspace::try_new().unwrap();
        let built_in_evidence = workspace.artifacts.evidence.digest();

        assert_eq!(
            workspace
                .import_cnc_source_bytes(CNC_EXAMPLE.as_bytes().to_vec())
                .unwrap(),
            CNC_EXAMPLE.len()
        );
        let report = workspace.artifacts.source.cnc_report().unwrap();
        let first_raw_source = report.raw_source_digest();
        assert_eq!(first_raw_source, sha256(CNC_EXAMPLE.as_bytes()).digest);
        assert_eq!(report.spans().len(), 2);
        assert_ne!(workspace.artifacts.evidence.digest(), built_in_evidence);
        let retained_evidence = workspace.artifacts.evidence.digest();
        let retained_exact_source = workspace.artifacts.evidence.source_digest();
        let retained_partition = workspace
            .artifacts
            .partition
            .publication()
            .object
            .content
            .digest;

        let comment_variant = CNC_EXAMPLE.replace(
            "selected exact geometry subset; feed/tool/process words are rejected",
            "different non-canonical comment text",
        );
        workspace
            .import_cnc_source_bytes(comment_variant.as_bytes().to_vec())
            .unwrap();
        assert_ne!(
            workspace
                .artifacts
                .source
                .cnc_report()
                .unwrap()
                .raw_source_digest(),
            first_raw_source
        );
        assert_eq!(workspace.artifacts.evidence.digest(), retained_evidence);
        assert_eq!(
            workspace.artifacts.evidence.source_digest(),
            retained_exact_source
        );
        assert_eq!(
            workspace
                .artifacts
                .partition
                .publication()
                .object
                .content
                .digest,
            retained_partition
        );
    }

    #[test]
    fn cnc_source_transaction_rejects_process_words_and_machine_travel_escape() {
        let mut workspace = MachineCamWorkspace::try_new().unwrap();
        workspace
            .import_cnc_source_bytes(CNC_EXAMPLE.as_bytes().to_vec())
            .unwrap();
        let retained_raw = workspace
            .artifacts
            .source
            .cnc_report()
            .unwrap()
            .raw_source_digest();
        let retained_evidence = workspace.artifacts.evidence.digest();

        let process_word = CNC_EXAMPLE.replace("X4 Y0", "X4 Y0 F100");
        let error = workspace
            .import_cnc_source_bytes(process_word.into_bytes())
            .unwrap_err();
        assert!(error.contains("F word"));
        assert_eq!(
            workspace
                .artifacts
                .source
                .cnc_report()
                .unwrap()
                .raw_source_digest(),
            retained_raw
        );

        let outside_travel = b"G21 G90 G17 G91.1\nG0 X0 Y0\nG1 X301 Y0\nM2\n";
        let error = workspace
            .import_cnc_source_bytes(outside_travel.to_vec())
            .unwrap_err();
        assert!(error.contains("travel boundary"), "{error}");
        assert_eq!(workspace.artifacts.evidence.digest(), retained_evidence);
        assert_eq!(
            workspace
                .artifacts
                .source
                .cnc_report()
                .unwrap()
                .raw_source_digest(),
            retained_raw
        );
    }

    #[test]
    fn headless_machine_cam_workspace_produces_a_complete_egui_frame() {
        let mut workspace = MachineCamWorkspace::try_new().unwrap();
        let context = egui::Context::default();
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_280.0, 1_600.0),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| workspace.show(ui));
            },
        );
        assert!(!output.shapes.is_empty());
        assert!(!output.textures_delta.set.is_empty());
    }
}
