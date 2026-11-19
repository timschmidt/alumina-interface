//! Bounded fuzz inputs exercised through real configuration, scheduling and replay.
use alumina_config::{
    ConfigurationDocumentView, ConfigurationRecord, Rational as WireRational, ScalarFact,
};
use alumina_interface_core::{
    MachineProfileError,
    machine_profile::{MachineDynamicsProfile2, MachineResolutionBudget2},
    motion_schedule::{
        ScheduledLoweringLimits, certify_jerk_schedule, lower_certified_schedule_to_v1,
    },
    partition::{MachinePartitionPolicy2, package_canonical_scheduled_program},
    schedule_evidence::{
        build_canonical_schedule_evidence, replay_canonical_schedule_evidence,
        verify_canonical_schedule_evidence_bytes,
    },
    toolpath::MetricPathApproximationLimits2,
};
use alumina_machine_ir::{BlockValidationLimits, ValidationLimits};
use alumina_storage::{CacheLimits, UploadId, sha256};
use hypercurve::{Curve2, CurveGeometry2, CurvePath2, LineSeg2, Point2};
use hyperreal::Rational;

const SEED: &[u8] = include_bytes!("../fixtures/machine.config");

/// Outcomes are counted separately; typed rejection is not completed lowering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    SkippedOversize,
    InvalidConfiguration,
    InvalidProfile,
    Profiled,
    BudgetRejected,
    ScheduleRejected,
    LoweringRejected,
    Replayed,
}
fn byte(data: &[u8], i: usize) -> u8 {
    data.get(i).copied().unwrap_or(0)
}
fn fraction(n: i64, d: u64) -> Rational {
    Rational::fraction(n, d).unwrap()
}

// Structured travel uncertainty as a fraction of the fixture's nominal travel.
fn relative_travel_radius(code: u8) -> (i64, u64) {
    if code & 128 == 0 {
        (i64::from(code), 1_000_000)
    } else {
        (i64::from(code & 15), 8)
    }
}

// Encode bounded nonnegative uncertainty in the wire format's reduced form.
fn wire_uncertainty(numerator: i64, denominator: u64) -> WireRational {
    let value = fraction(numerator, denominator);
    WireRational::new(
        i64::try_from(value.numerator()).unwrap(),
        u64::try_from(value.denominator()).unwrap(),
    )
    .unwrap()
}

fn configuration(data: &[u8], allow_raw: bool) -> Result<Vec<u8>, ProfileFailure> {
    if allow_raw && byte(data, 0) & 128 != 0 {
        let mut bytes = SEED.to_vec();
        let index = usize::from(u16::from_le_bytes([byte(data, 1), byte(data, 2)])) % bytes.len();
        bytes[index] ^= byte(data, 3).max(1);
        return Ok(bytes);
    }
    let view = ConfigurationDocumentView::decode::<32>(
        &board_mks_tinybee::PACKAGE,
        SEED,
        sha256(SEED).digest,
    )
    .unwrap();
    let mut bytes = view.header().encode().unwrap().to_vec();
    for record in view.records() {
        let mut record = record.unwrap();
        if let ConfigurationRecord::Scalar(scalar) = &mut record
            && scalar.instance == u16::from(byte(data, 1) & 1)
        {
            let calibration_mode = byte(data, 0) & 32 != 0;
            if scalar.fact == ScalarFact::AxisMicrosteps {
                let nominal = if byte(data, 0) & 64 != 0 {
                    0
                } else {
                    1 + i64::from(byte(data, 2) % 32)
                };
                scalar.value = WireRational::new(nominal, 1).unwrap();
            }
            if (scalar.fact == ScalarFact::AxisMicrosteps && !calibration_mode)
                || (scalar.fact == ScalarFact::AxisCalibrationScale && calibration_mode)
            {
                let scale = if calibration_mode { 1_000_000 } else { 1 };
                scalar.uncertainty = wire_uncertainty(
                    i64::from(byte(data, 3) % 8),
                    (1 + u64::from(byte(data, 4) % 4)) * scale,
                );
            }
            if scalar.fact == ScalarFact::AxisTravelMetresPerOutputTurn && byte(data, 10) != 0 {
                let (units, scale) = relative_travel_radius(byte(data, 10));
                scalar.uncertainty = wire_uncertainty(units, 500 * scale);
            }
        }
        bytes.extend_from_slice(&record.encode().map_err(|_| ProfileFailure::Configuration)?);
    }
    Ok(bytes)
}
/// Failure stage of a generated profile, retaining the rejected fact when available.
#[derive(Debug)]
pub enum ProfileFailure {
    Configuration,
    Profile(MachineProfileError),
}

impl From<ProfileFailure> for Outcome {
    fn from(failure: ProfileFailure) -> Self {
        match failure {
            ProfileFailure::Configuration => Self::InvalidConfiguration,
            ProfileFailure::Profile(_) => Self::InvalidProfile,
        }
    }
}

/// Check the exact profile derived from a bounded fuzz input. With raw mutations
/// enabled, the high bit of byte zero selects a single wire-byte edit.
pub fn profile(data: &[u8], allow_raw: bool) -> Result<MachineDynamicsProfile2, ProfileFailure> {
    let bytes = configuration(data, allow_raw)?;
    let digest = sha256(&bytes).digest;
    let view = ConfigurationDocumentView::decode::<32>(&board_mks_tinybee::PACKAGE, &bytes, digest)
        .map_err(|_| ProfileFailure::Configuration)?;
    let profile =
        MachineDynamicsProfile2::from_configuration(view).map_err(ProfileFailure::Profile)?;
    let fresh =
        ConfigurationDocumentView::decode::<32>(&board_mks_tinybee::PACKAGE, &bytes, digest)
            .unwrap();
    assert_eq!(
        profile,
        MachineDynamicsProfile2::from_configuration(fresh).unwrap()
    );
    assert_eq!(profile.configuration_digest(), digest);
    assert_eq!(
        profile.capability_digest(),
        board_mks_tinybee::CAPABILITY_DIGEST
    );
    if !allow_raw || byte(data, 0) & 128 == 0 {
        for (axis, p) in profile.axes().iter().enumerate() {
            let (m, u) = if axis == usize::from(byte(data, 1) & 1) {
                (
                    Rational::from(1 + i64::from(byte(data, 2) % 32)),
                    fraction(
                        i64::from(byte(data, 3) % 8),
                        1 + u64::from(byte(data, 4) % 4),
                    ),
                )
            } else {
                (Rational::from(16), Rational::zero())
            };
            // Independent constants from the retained fixture: 200 steps/turn,
            // 1/500 m/turn and calibration 1 +/- 1/1_000_000.
            let density = p.command_density_steps_per_metre();
            let nominal = &m * Rational::from(100_000);
            assert_eq!(density.nominal(), &nominal);
            let selected = axis == usize::from(byte(data, 1) & 1);
            let (lower, upper) = if selected && byte(data, 0) & 32 != 0 {
                let uncertainty = &u * fraction(1, 1_000_000);
                (
                    &nominal * (Rational::one() - &uncertainty),
                    &nominal * (Rational::one() + &uncertainty),
                )
            } else {
                (
                    (&m - &u) * fraction(999_999, 10),
                    (&m + &u) * fraction(1_000_001, 10),
                )
            };
            if selected && byte(data, 10) != 0 {
                let (units, scale) = relative_travel_radius(byte(data, 10));
                assert!(
                    units < scale as i64,
                    "travel interval must be strictly positive"
                );
                // Undo the reciprocal using the opposite travel endpoint.
                // Both products must recover the independent fixed-travel expectations.
                let radius = fraction(units, scale);
                assert_eq!(density.lower() * (Rational::one() + &radius), lower);
                assert_eq!(density.upper() * (Rational::one() - &radius), upper);
                let mm = p.command_density_steps_per_millimetre();
                let to_metres = Rational::from(1_000);
                assert_eq!(&(mm.lower() * &to_metres), density.lower());
                assert_eq!(&(mm.nominal() * &to_metres), density.nominal());
                assert_eq!(&(mm.upper() * &to_metres), density.upper());
            } else {
                assert_eq!(density.lower(), &lower);
                assert_eq!(density.upper(), &upper);
            }
        }
    }
    Ok(profile)
}
/// Mutate canonical scalar inputs or one wire byte, then replay exact profile facts.
pub fn machine_profile(data: &[u8]) -> Outcome {
    if data.len() > 64 {
        return Outcome::SkippedOversize;
    }
    match profile(data, true) {
        Ok(_) => Outcome::Profiled,
        Err(failure) => failure.into(),
    }
}
/// Exercise bounded straight paths through production lowering and cached execution.
pub fn machine_lowering(data: &[u8]) -> Outcome {
    if data.len() > 64 {
        return Outcome::SkippedOversize;
    }
    let profile = match profile(data, false) {
        Ok(p) => p,
        Err(failure) => return failure.into(),
    };
    let requested = if byte(data, 5) & 1 == 0 {
        fraction(1, 10)
    } else {
        fraction(1, 100_000)
    };
    let budget = match MachineResolutionBudget2::certify(
        &profile,
        requested,
        Rational::zero(),
        fraction(1, 100),
    ) {
        Ok(b) => b,
        Err(_) => return Outcome::BudgetRejected,
    };
    let source = CurvePath2::try_new(vec![Curve2::new(CurveGeometry2::Line(
        LineSeg2::try_new(
            Point2::from_values(0, 0),
            Point2::from_values(
                1 + i32::from(byte(data, 6) % 4),
                1 + i32::from(byte(data, 7) % 4),
            ),
        )
        .unwrap(),
    ))])
    .unwrap();
    let schedule = match certify_jerk_schedule(
        &source,
        &profile,
        &budget,
        MetricPathApproximationLimits2::try_new(8, 4).unwrap(),
    ) {
        Ok(s) => s,
        Err(_) => return Outcome::ScheduleRejected,
    };
    let limits =
        ScheduledLoweringLimits::try_new(if byte(data, 8) & 1 == 0 { 4096 } else { 2 }).unwrap();
    let lowered = match lower_certified_schedule_to_v1(
        &schedule,
        &profile,
        &budget,
        fraction(1, 1_000),
        limits,
    ) {
        Ok(p) => p,
        Err(_) => return Outcome::LoweringRejected,
    };
    assert_eq!(
        lowered.configuration_digest(),
        profile.configuration_digest()
    );
    assert!(
        lowered
            .points()
            .windows(2)
            .all(|p| p[0].tick() < p[1].tick())
    );
    assert!(
        lowered
            .segments()
            .windows(2)
            .all(|s| s[0].end_tick == s[1].start_tick)
    );
    let preflight = alumina_motion::preflight_stepper_segments(
        profile.stepper_timing(0),
        [0, 0],
        lowered.segments(),
    )
    .unwrap();
    assert_eq!(preflight, lowered.executor_preflight());
    let policy = MachinePartitionPolicy2::try_new(
        [0x41; 16],
        profile.capability_digest(),
        profile.configuration_digest(),
        BlockValidationLimits {
            maximum_block_ticks: 10_000_000,
            segment: ValidationLimits {
                maximum_segment_ticks: 10_000_000,
                maximum_steps_per_segment: 100_000,
            },
        },
        UploadId(0x1122_3344_5566_7788),
        700,
        CacheLimits {
            maximum_object_bytes: 4 * 1024 * 1024,
            maximum_chunk_bytes: 1024,
            maximum_chunks: 10_000,
        },
    )
    .unwrap();
    let partition = package_canonical_scheduled_program(&lowered, policy).unwrap();
    let replay = alumina_sim::motion::replay_cached_stepper_partition::<2>(
        partition.bytes(),
        partition.job_descriptor(0x8877).unwrap(),
        profile.stepper_timing(0),
    )
    .unwrap();
    assert_eq!(replay.block_count, partition.block_count());
    assert_eq!(replay.segment_count as usize, lowered.segments().len());
    let evidence = build_canonical_schedule_evidence(&schedule, &lowered, &partition).unwrap();
    replay_canonical_schedule_evidence(&evidence, &schedule, &lowered, &partition).unwrap();
    let again = build_canonical_schedule_evidence(&schedule, &lowered, &partition).unwrap();
    assert_eq!(evidence.encoded(), again.encoded());
    let mut altered = evidence.encoded().to_vec();
    let index = usize::from(byte(data, 9)) % altered.len();
    altered[index] ^= 1;
    assert!(
        verify_canonical_schedule_evidence_bytes(
            &altered,
            evidence.digest(),
            &schedule,
            &lowered,
            &partition
        )
        .is_err()
    );
    assert!(
        verify_canonical_schedule_evidence_bytes(
            &altered,
            sha256(&altered).digest,
            &schedule,
            &lowered,
            &partition
        )
        .is_err()
    );
    Outcome::Replayed
}
