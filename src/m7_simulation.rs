//! Deterministic end-to-end browser coordinator fixture for operator diagnostics.

use std::fmt;

use alumina_clock::{BOOT_ID_BYTES, BootId, ClockEstimationPolicy};
use alumina_interface_client::Response;
use alumina_interface_client::clock::DeviceClockModel;
use alumina_interface_client::schedule::ParticipantSchedulePhase;
use alumina_interface_core::CanonicalGlobalJob2;
use alumina_job::{
    JOB_COMMIT_ID_BYTES, JobCommitId, JobCommitRequest, JobDescriptor, JobScheduleAction,
    JobScheduleAdmission, JobScheduleReference, JobStartObservation, JobStartObservationSource,
    JobStatusReport, PreparedJobSchedule, RealtimeJobReport, RealtimeJobState, ServiceJobReport,
    ServiceJobState,
};
use alumina_machine_ir::StreamTick;
use alumina_protocol::{DeviceCycle, DeviceId, Digest, Operation, StatusCode};
use alumina_sim::distributed::AffineDeviceClock;
use alumina_storage::{ChunkUploadHeader, UploadId, UploadPhase, UploadPlan, UploadProgress};

use crate::cache_delivery::{
    ParticipantCachePhase, ParticipantCacheReady, prepare_global_cache_delivery,
};
use crate::distributed_schedule::{
    DistributedDeadlineWindow, DistributedScheduleCoordinator, DistributedSchedulePhase,
    ParticipantPreparation, ParticipantStartInput, ParticipantStartTiming,
};

const OBSERVATION_UI_NS: u64 = 3_001_000_000;
const CONFIRM_UI_NS: u64 = 3_100_000_000;
const TARGET_UI_NS: u64 = 5_000_000_000;
const RECONCILIATION_UI_NS: u64 = 5_100_000_000;
const CLOCK_TOLERANCE_CYCLES: u64 = 2_000;

/// One simulated MCU's exact clock and scheduled-start diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SimulatedParticipantDiagnostic {
    /// Stable device identity from the canonical global manifest.
    pub device_id: DeviceId,
    /// Simulated oscillator adjustment relative to its declared frequency.
    pub rate_adjustment_ppm: i32,
    /// Exact causal observations retained by the browser model.
    pub accepted_clock_samples: u32,
    /// Rejected observations retained as diagnostic evidence.
    pub rejected_clock_samples: u32,
    /// Radius of the selected future-cycle interval.
    pub clock_uncertainty_cycles: u64,
    /// Device-local cycle installed as the hardware release epoch.
    pub scheduled_cycle: DeviceCycle,
    /// Exact simulated UI time at which that local cycle is first reached.
    pub simulated_start_ui_ns: u64,
    /// Explicit authority carried by the canonical firmware report.
    pub observation_source: JobStartObservationSource,
    /// Conservative browser-time replay interval for the reported cycle.
    pub reconciled_earliest_ui_ns: u64,
    pub reconciled_latest_ui_ns: u64,
    /// Last exact participant phase reconciled through `JobStatus`.
    pub terminal_phase: ParticipantSchedulePhase,
}

/// Deterministic report rendered by the UI without claiming live hardware.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepresentativeM7SimulationReport {
    /// One shared future UI epoch used to derive all local start cycles.
    pub target_ui_ns: u64,
    /// Ordered global phases observed through the successful lifecycle.
    pub observed_global_phases: Vec<DistributedSchedulePhase>,
    /// Final global state after exact terminal status reconciliation.
    pub terminal_phase: DistributedSchedulePhase,
    /// Conservative deadline class before confirmation begins.
    pub confirmation_window: DistributedDeadlineWindow,
    /// Whether one delivered install with a lost response was reconciled by status.
    pub lost_install_reconciled: bool,
    /// Largest absolute simulated start displacement from the shared UI epoch.
    pub maximum_target_error_ns: u64,
    /// Difference between earliest and latest simulated participant start edges.
    pub simulated_edge_spread_ns: u64,
    /// Worst-case spread across the browser-replayed observation intervals.
    pub maximum_reconciled_edge_spread_ns: u64,
    /// Largest replay-interval endpoint displacement from the shared epoch.
    pub maximum_reconciled_target_error_ns: u64,
    /// Sorted participant diagnostics.
    pub participants: Vec<SimulatedParticipantDiagnostic>,
}

/// Runs cache delivery, clock acquisition, and the complete harmless schedule model.
///
/// # Errors
///
/// Returns a named fail-closed stage if any canonical client, clock, cache,
/// schedule, arithmetic, or simulator invariant rejects the representative job.
pub fn run_representative_m7_simulation(
    job: &CanonicalGlobalJob2,
) -> Result<RepresentativeM7SimulationReport, RepresentativeM7SimulationError> {
    let cache_ready = simulate_cache_delivery(job)?;
    let clock_set = simulate_clock_models(job)?;
    let mut coordinator =
        DistributedScheduleCoordinator::after_cache(job, &cache_ready, &clock_set.preparations)
            .map_err(|_| stage("bind cache-ready participant set"))?;
    let mut observed_global_phases = vec![coordinator.phase()];
    let mut authorities = prepare_all(job, &clock_set.clocks, &mut coordinator)?;
    observed_global_phases.push(coordinator.phase());

    let inputs = start_inputs(job, &clock_set.models)?;
    coordinator
        .bind_start(OBSERVATION_UI_NS, TARGET_UI_NS, &inputs)
        .map_err(|_| stage("bind shared future start"))?;
    let lost_install_reconciled =
        install_all_with_one_lost_response(&mut coordinator, &mut authorities)?;
    observed_global_phases.push(coordinator.phase());

    let confirmation_window = coordinator
        .deadline_window(CONFIRM_UI_NS, &inputs)
        .map_err(|_| stage("classify confirmation deadline"))?;
    if confirmation_window != DistributedDeadlineWindow::ConfirmationOpen {
        return Err(stage("confirmation window unexpectedly closed"));
    }
    coordinator
        .begin_confirmation(CONFIRM_UI_NS, &inputs)
        .map_err(|_| stage("open confirmation phase"))?;
    confirm_all(&mut coordinator, &mut authorities)?;
    observed_global_phases.push(coordinator.phase());

    let simulated_start_ui_ns = execute_all(
        &mut coordinator,
        &mut authorities,
        &mut observed_global_phases,
    )?;
    build_report(
        &coordinator,
        &authorities,
        &inputs,
        &simulated_start_ui_ns,
        observed_global_phases,
        confirmation_window,
        lost_install_reconciled,
    )
}

fn build_report(
    coordinator: &DistributedScheduleCoordinator,
    authorities: &[SimulatedAuthority],
    inputs: &[ParticipantStartInput<'_>],
    simulated_start_ui_ns: &[u64],
    observed_global_phases: Vec<DistributedSchedulePhase>,
    confirmation_window: DistributedDeadlineWindow,
    lost_install_reconciled: bool,
) -> Result<RepresentativeM7SimulationReport, RepresentativeM7SimulationError> {
    let reconciliation = coordinator
        .reconcile_start_observations(RECONCILIATION_UI_NS, inputs, 2_000_000)
        .map_err(|_| stage("reconcile simulated start observations"))?;
    let earliest = simulated_start_ui_ns
        .iter()
        .copied()
        .min()
        .ok_or_else(|| stage("observe participant starts"))?;
    let latest = simulated_start_ui_ns
        .iter()
        .copied()
        .max()
        .ok_or_else(|| stage("observe participant starts"))?;
    let maximum_target_error_ns = simulated_start_ui_ns
        .iter()
        .map(|time| time.abs_diff(TARGET_UI_NS))
        .max()
        .ok_or_else(|| stage("measure target error"))?;

    let mut participants = Vec::new();
    participants
        .try_reserve_exact(authorities.len())
        .map_err(|_| stage("allocate participant diagnostics"))?;
    for (index, authority) in authorities.iter().enumerate() {
        let model = inputs
            .iter()
            .find(|input| input.device_id == authority.device_id)
            .map(|input| input.clock)
            .ok_or_else(|| stage("align report clock"))?;
        let commit = authority
            .commit
            .ok_or_else(|| stage("report participant commit"))?;
        let observed = reconciliation
            .participants
            .get(index)
            .filter(|observed| observed.device_id == authority.device_id)
            .ok_or_else(|| stage("align reconciled start observation"))?;
        if !(observed.earliest_ui_ns..=observed.latest_ui_ns)
            .contains(&simulated_start_ui_ns[index])
        {
            return Err(stage("contain exact simulated start in replay interval"));
        }
        participants.push(SimulatedParticipantDiagnostic {
            device_id: authority.device_id,
            rate_adjustment_ppm: authority.clock.rate_adjustment_ppm,
            accepted_clock_samples: model.accepted_samples(),
            rejected_clock_samples: model.rejected_samples(),
            clock_uncertainty_cycles: commit.clock_uncertainty_cycles,
            scheduled_cycle: commit.local_start_cycle,
            simulated_start_ui_ns: simulated_start_ui_ns[index],
            observation_source: observed.source,
            reconciled_earliest_ui_ns: observed.earliest_ui_ns,
            reconciled_latest_ui_ns: observed.latest_ui_ns,
            terminal_phase: coordinator
                .participant_phase(authority.device_id)
                .ok_or_else(|| stage("report participant phase"))?,
        });
    }

    Ok(RepresentativeM7SimulationReport {
        target_ui_ns: TARGET_UI_NS,
        observed_global_phases,
        terminal_phase: coordinator.phase(),
        confirmation_window,
        lost_install_reconciled,
        maximum_target_error_ns,
        simulated_edge_spread_ns: latest - earliest,
        maximum_reconciled_edge_spread_ns: reconciliation.maximum_edge_spread_ns,
        maximum_reconciled_target_error_ns: reconciliation.maximum_target_error_ns,
        participants,
    })
}

fn execute_all(
    coordinator: &mut DistributedScheduleCoordinator,
    authorities: &mut [SimulatedAuthority],
    observed_global_phases: &mut Vec<DistributedSchedulePhase>,
) -> Result<Vec<u64>, RepresentativeM7SimulationError> {
    prime_all(authorities)?;
    reconcile_status_round(coordinator, authorities, false)?;
    observed_global_phases.push(coordinator.phase());

    let mut simulated_start_ui_ns = Vec::new();
    simulated_start_ui_ns
        .try_reserve_exact(authorities.len())
        .map_err(|_| stage("allocate start observations"))?;
    for (index, authority) in authorities.iter_mut().enumerate() {
        let commit = authority
            .commit
            .ok_or_else(|| stage("retain installed commit"))?;
        if !matches!(
            authority.schedule.advance(commit.local_start_cycle),
            JobScheduleAction::Start { .. }
        ) {
            return Err(stage("emit simulated scheduled start"));
        }
        simulated_start_ui_ns.push(
            authority
                .clock
                .ui_ns_at_or_after_cycle(commit.local_start_cycle)
                .map_err(|_| stage("invert simulated device clock"))?,
        );
        let output_token = u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or_else(|| stage("construct simulated output token"))?;
        authority
            .schedule
            .record_start_observation(JobStartObservation {
                source: JobStartObservationSource::SimulatedLatch,
                output_token,
                scheduled_cycle: commit.local_start_cycle,
                earliest_cycle: commit.local_start_cycle,
                latest_cycle: commit.local_start_cycle,
            })
            .map_err(|_| stage("record simulated start observation"))?;
    }
    reconcile_status_round(coordinator, authorities, false)?;

    for authority in &mut *authorities {
        let commit = authority
            .commit
            .ok_or_else(|| stage("retain running commit"))?;
        let complete_cycle = commit
            .local_start_cycle
            .0
            .checked_add(authority.execution_cycles)
            .map(DeviceCycle)
            .ok_or_else(|| stage("advance terminal cycle"))?;
        authority
            .schedule
            .complete(complete_cycle)
            .map_err(|_| stage("complete simulated schedule"))?;
    }
    reconcile_status_round(coordinator, authorities, true)?;
    observed_global_phases.push(coordinator.phase());
    Ok(simulated_start_ui_ns)
}

fn simulate_cache_delivery(
    job: &CanonicalGlobalJob2,
) -> Result<Vec<ParticipantCacheReady>, RepresentativeM7SimulationError> {
    let upload_ids: Vec<_> = (0..job.participants().len())
        .map(|index| {
            u64::try_from(index)
                .ok()
                .and_then(|index| 0x7172_7374_7576_7701_u64.checked_add(index))
                .map(UploadId)
                .ok_or_else(|| stage("construct cache upload identity"))
        })
        .collect::<Result<_, _>>()?;
    let mut deliveries = prepare_global_cache_delivery(job, &upload_ids)
        .map_err(|_| stage("construct participant cache deliveries"))?;
    let mut ready = Vec::new();
    ready
        .try_reserve_exact(deliveries.len())
        .map_err(|_| stage("allocate cache readiness"))?;

    for delivery in &mut deliveries {
        let mut current_plan: Option<UploadPlan> = None;
        let mut accepted_bytes = 0_u64;
        while delivery.phase() != ParticipantCachePhase::Complete {
            let operation = delivery
                .next_request(job)
                .map_err(|_| stage("emit simulated cache operation"))?
                .ok_or_else(|| stage("cache operation ended before readiness"))?;
            let response = match operation.operation {
                Operation::StorageInspect => Response {
                    status: StatusCode::NotFound,
                    body: Vec::new(),
                },
                Operation::StorageBeginUpload => {
                    let plan = UploadPlan::decode(&operation.body, job.policy().cache_limits())
                        .map_err(|_| stage("decode simulated upload plan"))?;
                    current_plan = Some(plan);
                    accepted_bytes = 0;
                    upload_progress(plan, 0, accepted_bytes)
                }
                Operation::StoragePutChunk => {
                    let plan = current_plan.ok_or_else(|| stage("retain simulated upload plan"))?;
                    let header = ChunkUploadHeader::decode(
                        operation
                            .body
                            .get(..ChunkUploadHeader::WIRE_LEN)
                            .ok_or_else(|| stage("decode simulated chunk header"))?,
                    )
                    .map_err(|_| stage("decode simulated chunk header"))?;
                    accepted_bytes = accepted_bytes
                        .checked_add(u64::from(header.byte_len))
                        .ok_or_else(|| stage("advance simulated upload bytes"))?;
                    let next_chunk = header
                        .index
                        .checked_add(1)
                        .ok_or_else(|| stage("advance simulated upload chunk"))?;
                    upload_progress(plan, next_chunk, accepted_bytes)
                }
                Operation::StorageFinalize => Response {
                    status: StatusCode::Ok,
                    body: Vec::new(),
                },
                _ => return Err(stage("unexpected simulated cache operation")),
            };
            delivery
                .accept_response(&response)
                .map_err(|_| stage("accept simulated cache response"))?;
        }
        ready.push(
            delivery
                .readiness()
                .ok_or_else(|| stage("publish simulated cache readiness"))?,
        );
    }
    Ok(ready)
}

fn upload_progress(plan: UploadPlan, next_chunk: u32, accepted_bytes: u64) -> Response {
    Response {
        status: StatusCode::Ok,
        body: UploadProgress {
            upload_id: plan.upload_id,
            phase: UploadPhase::Receiving,
            next_chunk,
            accepted_bytes,
            total_bytes: plan.object.byte_len,
        }
        .encode()
        .to_vec(),
    }
}

struct SimulatedClockSet {
    clocks: Vec<AffineDeviceClock>,
    models: Vec<DeviceClockModel>,
    preparations: Vec<ParticipantPreparation>,
}

fn simulate_clock_models(
    job: &CanonicalGlobalJob2,
) -> Result<SimulatedClockSet, RepresentativeM7SimulationError> {
    let mut clocks = Vec::new();
    let mut models = Vec::new();
    let mut preparations = Vec::new();
    clocks
        .try_reserve_exact(job.participants().len())
        .map_err(|_| stage("allocate simulated clocks"))?;
    models
        .try_reserve_exact(job.participants().len())
        .map_err(|_| stage("allocate browser clock models"))?;
    preparations
        .try_reserve_exact(job.participants().len())
        .map_err(|_| stage("allocate preparation identities"))?;

    for (index, participant) in job.participants().iter().enumerate() {
        let index_u8 = u8::try_from(index).map_err(|_| stage("index simulated participant"))?;
        let index_u64 = u64::try_from(index).map_err(|_| stage("index simulated participant"))?;
        let boot_id = BootId::new([0x31_u8.wrapping_add(index_u8); BOOT_ID_BYTES])
            .map_err(|_| stage("construct simulated boot identity"))?;
        let clock = AffineDeviceClock {
            boot_id,
            nominal_frequency_hz: 1_000_000,
            rate_adjustment_ppm: if index % 2 == 0 { 40 } else { -35 },
            offset_cycles: 75_000_u64
                .checked_add(
                    index_u64
                        .checked_mul(250_000)
                        .ok_or_else(|| stage("construct simulated clock offset"))?,
                )
                .ok_or_else(|| stage("construct simulated clock offset"))?,
            minimum_lead_cycles: 100_000,
            maximum_schedule_horizon_cycles: 10_000_000,
        };
        let model = simulate_clock_model(clock, index % 2 != 0)?;
        clocks.push(clock);
        models.push(model);
        preparations.push(ParticipantPreparation {
            device_id: participant.device_id(),
            boot_id,
            prepare_id: 100_u64
                .checked_add(index_u64)
                .ok_or_else(|| stage("construct preparation identity"))?,
        });
    }
    Ok(SimulatedClockSet {
        clocks,
        models,
        preparations,
    })
}

fn simulate_clock_model(
    clock: AffineDeviceClock,
    reverse_delays: bool,
) -> Result<DeviceClockModel, RepresentativeM7SimulationError> {
    let mut model = DeviceClockModel::new(ClockEstimationPolicy {
        maximum_round_trip_ns: 3_000_000,
        maximum_device_processing_cycles: 1_000,
        maximum_drift_ppm: 100,
        maximum_sample_age_ns: 10_000_000_000,
        maximum_schedule_horizon_ns: 10_000_000_000,
        minimum_schedule_lead_ns: 100_000_000,
        minimum_samples: 3,
    })
    .map_err(|_| stage("construct browser clock policy"))?;
    let forward = [
        (120_000, 50_000, 700_000),
        (650_000, 60_000, 120_000),
        (300_000, 40_000, 600_000),
    ];
    let reverse = [
        (700_000, 40_000, 120_000),
        (100_000, 60_000, 700_000),
        (500_000, 50_000, 400_000),
    ];
    let delays = if reverse_delays { reverse } else { forward };
    for (index, (up, processing, down)) in delays.into_iter().enumerate() {
        let send = u64::try_from(index + 1)
            .map_err(|_| stage("construct simulated probe time"))?
            .checked_mul(1_000_000_000)
            .ok_or_else(|| stage("construct simulated probe time"))?;
        let request = model
            .begin_probe(send)
            .map_err(|_| stage("begin simulated clock probe"))?;
        let observation = clock
            .heartbeat(
                request.probe_id(),
                request.ui_send_ns(),
                up,
                processing,
                down,
            )
            .map_err(|_| stage("simulate asymmetric heartbeat"))?;
        model
            .accept_response(
                &Response {
                    status: StatusCode::Ok,
                    body: observation
                        .response
                        .encode()
                        .map_err(|_| stage("encode simulated heartbeat"))?
                        .to_vec(),
                },
                observation.ui_receive_ns,
            )
            .map_err(|_| stage("accept simulated heartbeat"))?;
    }
    Ok(model)
}

#[derive(Clone, Copy)]
struct SimulatedAuthority {
    device_id: DeviceId,
    clock: AffineDeviceClock,
    descriptor: JobDescriptor,
    terminal_progress: (StreamTick, Digest),
    execution_cycles: u64,
    schedule: PreparedJobSchedule,
    commit: Option<JobCommitRequest>,
}

fn prepare_all(
    job: &CanonicalGlobalJob2,
    clocks: &[AffineDeviceClock],
    coordinator: &mut DistributedScheduleCoordinator,
) -> Result<Vec<SimulatedAuthority>, RepresentativeM7SimulationError> {
    let mut authorities = Vec::new();
    authorities
        .try_reserve_exact(job.participants().len())
        .map_err(|_| stage("allocate simulated authorities"))?;
    while let Some(request) = coordinator
        .next_request()
        .map_err(|_| stage("emit simulated prepare"))?
    {
        if request.request.operation != Operation::JobPrepare {
            return Err(stage("unexpected simulated prepare operation"));
        }
        let descriptor = JobDescriptor::decode::<2>(&request.request.body)
            .map_err(|_| stage("decode simulated descriptor"))?;
        let index = job
            .participants()
            .binary_search_by_key(
                &request.device_id,
                alumina_interface_core::MachineJobParticipantPackage2::device_id,
            )
            .map_err(|_| stage("locate simulated participant"))?;
        let terminal = job.participants()[index].partition().terminal_progress();
        let execution_cycles = ceil_product_ratio(
            terminal.end_tick.0,
            clocks[index].nominal_frequency_hz,
            job.participants()[index].partition().local_timer_hz(),
        )?;
        let authority = SimulatedAuthority {
            device_id: request.device_id,
            clock: clocks[index],
            descriptor,
            terminal_progress: (terminal.end_tick, terminal.block_digest),
            execution_cycles,
            schedule: PreparedJobSchedule::prepare::<2>(clocks[index].boot_id, descriptor)
                .map_err(|_| stage("prepare simulated authority"))?,
            commit: None,
        };
        coordinator
            .accept_response(request.device_id, &status_response(&authority, false)?)
            .map_err(|_| stage("accept simulated prepare response"))?;
        authorities.push(authority);
    }
    authorities.sort_unstable_by_key(|authority| authority.device_id);
    if coordinator.phase() != DistributedSchedulePhase::Ready {
        return Err(stage("reach simulated ready phase"));
    }
    Ok(authorities)
}

fn start_inputs<'a>(
    job: &CanonicalGlobalJob2,
    clocks: &'a [DeviceClockModel],
) -> Result<Vec<ParticipantStartInput<'a>>, RepresentativeM7SimulationError> {
    job.participants()
        .iter()
        .enumerate()
        .map(|(index, participant)| {
            let byte = 0x81_u8
                .checked_add(u8::try_from(index).map_err(|_| stage("index start input"))?)
                .ok_or_else(|| stage("construct commit identity"))?;
            Ok(ParticipantStartInput {
                device_id: participant.device_id(),
                clock: clocks
                    .get(index)
                    .ok_or_else(|| stage("align start clock"))?,
                timing: ParticipantStartTiming {
                    maximum_uncertainty_cycles: CLOCK_TOLERANCE_CYCLES,
                    required_sync_tolerance_cycles: CLOCK_TOLERANCE_CYCLES,
                    confirmation_lead_cycles: 400_000,
                    abort_guard_lead_cycles: 200_000,
                    lease_cycles: 10_000_000,
                    commit_id: JobCommitId::new([byte; JOB_COMMIT_ID_BYTES])
                        .map_err(|_| stage("construct commit identity"))?,
                },
            })
        })
        .collect()
}

fn install_all_with_one_lost_response(
    coordinator: &mut DistributedScheduleCoordinator,
    authorities: &mut [SimulatedAuthority],
) -> Result<bool, RepresentativeM7SimulationError> {
    let mut lost_response = false;
    while coordinator.phase() != DistributedSchedulePhase::Installed {
        let request = coordinator
            .next_request()
            .map_err(|_| stage("emit simulated install/reconciliation"))?
            .ok_or_else(|| stage("install reconciliation ended early"))?;
        let authority = authority_mut(authorities, request.device_id)?;
        match request.request.operation {
            Operation::JobCommit => {
                let commit = JobCommitRequest::decode(&request.request.body)
                    .map_err(|_| stage("decode simulated commit"))?;
                authority
                    .schedule
                    .install(commit, admission(authority)?)
                    .map_err(|_| stage("install simulated commit"))?;
                authority.commit = Some(commit);
                if !lost_response {
                    if !coordinator
                        .abandon_pending(request.device_id)
                        .map_err(|_| stage("abandon simulated install response"))?
                    {
                        return Err(stage("lose simulated install response"));
                    }
                    lost_response = true;
                    continue;
                }
            }
            Operation::JobStatus => {}
            _ => return Err(stage("unexpected simulated install operation")),
        }
        coordinator
            .accept_response(request.device_id, &status_response(authority, false)?)
            .map_err(|_| stage("accept simulated install response"))?;
    }
    Ok(lost_response)
}

fn confirm_all(
    coordinator: &mut DistributedScheduleCoordinator,
    authorities: &mut [SimulatedAuthority],
) -> Result<(), RepresentativeM7SimulationError> {
    while let Some(request) = coordinator
        .next_request()
        .map_err(|_| stage("emit simulated confirmation"))?
    {
        if request.request.operation != Operation::JobConfirm {
            return Err(stage("unexpected simulated confirmation operation"));
        }
        let authority = authority_mut(authorities, request.device_id)?;
        let reference = JobScheduleReference::decode(&request.request.body)
            .map_err(|_| stage("decode simulated confirmation"))?;
        let now = authority
            .clock
            .cycle_at_ui_ns(CONFIRM_UI_NS)
            .map_err(|_| stage("sample simulated confirmation time"))?;
        authority
            .schedule
            .confirm(reference, now)
            .map_err(|_| stage("confirm simulated commit"))?;
        coordinator
            .accept_response(request.device_id, &status_response(authority, false)?)
            .map_err(|_| stage("accept simulated confirmation response"))?;
    }
    if coordinator.phase() != DistributedSchedulePhase::Confirmed {
        return Err(stage("reach simulated confirmed phase"));
    }
    Ok(())
}

fn prime_all(
    authorities: &mut [SimulatedAuthority],
) -> Result<(), RepresentativeM7SimulationError> {
    for authority in authorities {
        let commit = authority
            .commit
            .ok_or_else(|| stage("retain commit for priming"))?;
        if !matches!(
            authority.schedule.advance(commit.abort_guard_cycle),
            JobScheduleAction::PrimeHardware { .. }
        ) {
            return Err(stage("begin simulated hardware priming"));
        }
        authority
            .schedule
            .mark_primed(DeviceCycle(
                commit
                    .abort_guard_cycle
                    .0
                    .checked_add(1)
                    .ok_or_else(|| stage("advance simulated priming cycle"))?,
            ))
            .map_err(|_| stage("acknowledge simulated hardware horizon"))?;
    }
    Ok(())
}

fn reconcile_status_round(
    coordinator: &mut DistributedScheduleCoordinator,
    authorities: &[SimulatedAuthority],
    terminal: bool,
) -> Result<(), RepresentativeM7SimulationError> {
    let requests = coordinator
        .begin_status_round()
        .map_err(|_| stage("begin simulated status round"))?;
    for request in requests {
        if request.request.operation != Operation::JobStatus {
            return Err(stage("unexpected simulated status operation"));
        }
        let authority = authority(authorities, request.device_id)?;
        coordinator
            .accept_response(request.device_id, &status_response(authority, terminal)?)
            .map_err(|_| stage("accept simulated status response"))?;
    }
    Ok(())
}

fn admission(
    authority: &SimulatedAuthority,
) -> Result<JobScheduleAdmission, RepresentativeM7SimulationError> {
    Ok(JobScheduleAdmission {
        now: authority
            .clock
            .cycle_at_ui_ns(OBSERVATION_UI_NS)
            .map_err(|_| stage("sample simulated install time"))?,
        active_config: authority.descriptor.config_digest,
        minimum_lead_cycles: authority.clock.minimum_lead_cycles,
        maximum_start_horizon_cycles: authority.clock.maximum_schedule_horizon_cycles,
        maximum_lease_cycles: 20_000_000,
        maximum_sync_tolerance_cycles: CLOCK_TOLERANCE_CYCLES,
        minimum_prime_lead_cycles: 100_000,
        cache_ready: true,
        safety_ready: true,
        autonomous_allowed: false,
    })
}

fn status_response(
    authority: &SimulatedAuthority,
    terminal: bool,
) -> Result<Response, RepresentativeM7SimulationError> {
    let total = authority.descriptor.block_count;
    let retained = total.min(2);
    let (service_state, realtime_state, completed, progress, outstanding) = if terminal {
        (
            ServiceJobState::Complete,
            RealtimeJobState::Complete,
            total,
            Some(authority.terminal_progress),
            false,
        )
    } else {
        (
            ServiceJobState::Prefetching,
            RealtimeJobState::Admitted,
            0,
            None,
            true,
        )
    };
    let report = JobStatusReport {
        service: Some(ServiceJobReport {
            prepare_id: authority.descriptor.prepare_id,
            state: service_state,
            axis_count: authority.descriptor.axis_count,
            validated_blocks: if terminal { total } else { retained },
            sent_blocks: if terminal { total } else { retained },
            total_blocks: total,
            storage_chunks_read: total,
            queue_free: 0,
            queue_depth: 0,
            final_progress: progress,
        }),
        realtime: Some(RealtimeJobReport {
            prepare_id: authority.descriptor.prepare_id,
            state: realtime_state,
            admitted_blocks: if terminal { total } else { retained },
            completed_blocks: completed,
            total_blocks: total,
            queue_depth: 0,
            admitted_progress: if outstanding {
                Some((StreamTick(10), Digest([0x91; 32])))
            } else {
                None
            },
            completed_progress: progress,
            outstanding,
        }),
        schedule: Some(authority.schedule.report()),
    };
    Ok(Response {
        status: StatusCode::Ok,
        body: report
            .encode()
            .map_err(|_| stage("encode simulated job status"))?
            .to_vec(),
    })
}

fn authority(
    authorities: &[SimulatedAuthority],
    device_id: DeviceId,
) -> Result<&SimulatedAuthority, RepresentativeM7SimulationError> {
    authorities
        .binary_search_by_key(&device_id, |authority| authority.device_id)
        .map(|index| &authorities[index])
        .map_err(|_| stage("locate simulated authority"))
}

fn authority_mut(
    authorities: &mut [SimulatedAuthority],
    device_id: DeviceId,
) -> Result<&mut SimulatedAuthority, RepresentativeM7SimulationError> {
    authorities
        .binary_search_by_key(&device_id, |authority| authority.device_id)
        .map(|index| &mut authorities[index])
        .map_err(|_| stage("locate simulated authority"))
}

fn ceil_product_ratio(
    value: u64,
    multiplier: u64,
    divisor: u64,
) -> Result<u64, RepresentativeM7SimulationError> {
    if divisor == 0 {
        return Err(stage("convert simulated execution duration"));
    }
    let numerator = u128::from(value)
        .checked_mul(u128::from(multiplier))
        .ok_or_else(|| stage("convert simulated execution duration"))?;
    let divisor = u128::from(divisor);
    let rounded = numerator
        .checked_add(divisor - 1)
        .ok_or_else(|| stage("convert simulated execution duration"))?
        / divisor;
    u64::try_from(rounded).map_err(|_| stage("convert simulated execution duration"))
}

const fn stage(name: &'static str) -> RepresentativeM7SimulationError {
    RepresentativeM7SimulationError { stage: name }
}

/// Named fail-closed stage from the deterministic operator fixture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepresentativeM7SimulationError {
    stage: &'static str,
}

impl RepresentativeM7SimulationError {
    /// Static lifecycle stage that rejected the representative fixture.
    #[must_use]
    pub const fn stage(self) -> &'static str {
        self.stage
    }
}

impl fmt::Display for RepresentativeM7SimulationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "representative M7 simulation failed while {}",
            self.stage
        )
    }
}

impl std::error::Error for RepresentativeM7SimulationError {}

#[cfg(test)]
mod tests {
    use alumina_interface_core::{
        compile_representative_global_job, compile_representative_program,
    };

    use super::*;

    #[test]
    fn representative_flow_is_repeatable_and_reconciles_lost_install() {
        let program = compile_representative_program().unwrap();
        let job = compile_representative_global_job(&program).unwrap();
        let first = run_representative_m7_simulation(&job).unwrap();
        let second = run_representative_m7_simulation(&job).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.participants.len(), job.participants().len());
        assert_eq!(first.terminal_phase, DistributedSchedulePhase::Complete);
        assert_eq!(
            first.confirmation_window,
            DistributedDeadlineWindow::ConfirmationOpen
        );
        assert!(first.lost_install_reconciled);
        assert!(first.maximum_target_error_ns <= 2_000_000);
        assert!(first.simulated_edge_spread_ns <= 4_000_000);
        assert!(first.maximum_reconciled_target_error_ns <= 2_000_000);
        assert!(first.maximum_reconciled_edge_spread_ns <= 4_000_000);
        assert!(first.participants.iter().all(|participant| {
            participant.accepted_clock_samples == 3
                && participant.rejected_clock_samples == 0
                && participant.observation_source == JobStartObservationSource::SimulatedLatch
                && participant.terminal_phase == ParticipantSchedulePhase::Complete
        }));
    }
}
