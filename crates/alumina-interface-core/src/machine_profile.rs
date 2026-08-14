//! Exact machine dynamics and resolution facts derived from canonical configuration.
//!
//! This module does not accept a second UI-local machine schema. Its only input
//! is a [`ConfigurationDocumentView`] that already passed the firmware's board,
//! resource, semantic, and SHA-256 validation. Physical uncertainty remains an
//! interval until a conservative bound is selected for scheduling or error
//! allocation.

use std::cmp::Ordering;
use std::error::Error as StdError;
use std::fmt;

use alumina_config::{
    AxisDriverControl, BindingRole, ConfigurationDocumentView, ConfigurationError,
    ConfigurationIdentity, ExactScalar, ResourceBinding, ScalarFact,
};
use alumina_motion::{AxisTiming, StepperTiming};
use alumina_protocol::Digest;
use hyperlimit::{PredicatePolicy, compare_reals};
use hyperreal::{Problem, Rational, Real};

/// Result type for canonical machine-profile derivation.
pub type MachineProfileResult<T> = Result<T, MachineProfileError>;

/// Exact nominal value and closed uncertainty interval in the fact's units.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactInterval {
    nominal: Rational,
    lower: Rational,
    upper: Rational,
}

impl ExactInterval {
    fn from_scalar(scalar: ExactScalar) -> MachineProfileResult<Self> {
        let nominal = convert_rational(scalar.value)?;
        let uncertainty = convert_rational(scalar.uncertainty)?;
        Ok(Self {
            lower: &nominal - &uncertainty,
            upper: &nominal + &uncertainty,
            nominal,
        })
    }

    fn require_positive_lower(self, instance: u16, fact: ScalarFact) -> MachineProfileResult<Self> {
        if self.lower <= Rational::zero() {
            return Err(MachineProfileError::NonPositiveLowerBound { instance, fact });
        }
        Ok(self)
    }

    fn divided_by_positive_integer(&self, divisor: u64) -> Self {
        let divisor = Rational::from(divisor);
        Self {
            nominal: &self.nominal / &divisor,
            lower: &self.lower / &divisor,
            upper: &self.upper / divisor,
        }
    }

    /// Exact nominal value.
    pub const fn nominal(&self) -> &Rational {
        &self.nominal
    }

    /// Inclusive conservative lower endpoint.
    pub const fn lower(&self) -> &Rational {
        &self.lower
    }

    /// Inclusive conservative upper endpoint.
    pub const fn upper(&self) -> &Rational {
        &self.upper
    }
}

/// Exact resource, transmission, uncertainty, and dynamics facts for one axis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepperAxisMachineProfile {
    instance: u16,
    step: ResourceBinding,
    direction: ResourceBinding,
    driver_control: ResourceBinding,
    driver_control_action: AxisDriverControl,
    full_steps_per_turn: ExactInterval,
    microsteps: ExactInterval,
    motor_turns_per_output_turn: ExactInterval,
    travel_metres_per_output_turn: ExactInterval,
    calibration_scale: ExactInterval,
    command_density_steps_per_metre: ExactInterval,
    command_density_steps_per_millimetre: ExactInterval,
    position_minimum_metres: ExactInterval,
    position_maximum_metres: ExactInterval,
    usable_position_minimum_metres: Rational,
    usable_position_maximum_metres: Rational,
    configured_velocity_limit_metres_per_second: ExactInterval,
    effective_step_frequency_hz: Rational,
    step_rate_velocity_limit_metres_per_second: Rational,
    effective_velocity_limit_metres_per_second: Rational,
    acceleration_limit_metres_per_second_squared: ExactInterval,
    effective_acceleration_limit_metres_per_second_squared: Rational,
    jerk_limit_metres_per_second_cubed: ExactInterval,
    effective_jerk_limit_metres_per_second_cubed: Rational,
    following_error_metres: ExactInterval,
    maximum_following_error_metres: Rational,
}

impl StepperAxisMachineProfile {
    fn derive(
        view: ConfigurationDocumentView<'_>,
        instance: u16,
        timer_ticks_per_second: u64,
    ) -> MachineProfileResult<Self> {
        let step = required_binding(view, instance, BindingRole::AxisStep)?;
        let direction = required_binding(view, instance, BindingRole::AxisDirection)?;
        let enable = view.binding(instance, BindingRole::AxisEnable)?;
        let disable = view.binding(instance, BindingRole::AxisDisable)?;
        let (driver_control, driver_control_action) = match (enable, disable) {
            (Some(binding), None) => (binding, AxisDriverControl::Enable),
            (None, Some(binding)) => (binding, AxisDriverControl::Disable),
            _ => return Err(MachineProfileError::InvalidDriverControl { instance }),
        };

        let full_steps_per_turn =
            positive_scalar_interval(view, instance, ScalarFact::AxisFullStepsPerTurn)?;
        let microsteps = positive_scalar_interval(view, instance, ScalarFact::AxisMicrosteps)?;
        let motor_turns_per_output_turn =
            positive_scalar_interval(view, instance, ScalarFact::AxisMotorTurnsPerOutputTurn)?;
        let travel_metres_per_output_turn =
            positive_scalar_interval(view, instance, ScalarFact::AxisTravelMetresPerOutputTurn)?;
        let calibration_scale =
            positive_scalar_interval(view, instance, ScalarFact::AxisCalibrationScale)?;

        let numerator_nominal = &full_steps_per_turn.nominal
            * &microsteps.nominal
            * &motor_turns_per_output_turn.nominal
            * &calibration_scale.nominal;
        let numerator_lower = &full_steps_per_turn.lower
            * &microsteps.lower
            * &motor_turns_per_output_turn.lower
            * &calibration_scale.lower;
        let numerator_upper = &full_steps_per_turn.upper
            * &microsteps.upper
            * &motor_turns_per_output_turn.upper
            * &calibration_scale.upper;
        let command_density_steps_per_metre = ExactInterval {
            nominal: numerator_nominal / &travel_metres_per_output_turn.nominal,
            lower: numerator_lower / &travel_metres_per_output_turn.upper,
            upper: numerator_upper / &travel_metres_per_output_turn.lower,
        };
        let command_density_steps_per_millimetre =
            command_density_steps_per_metre.divided_by_positive_integer(1_000);

        let position_minimum_metres =
            scalar_interval(view, instance, ScalarFact::AxisPositionMinimumMetres)?;
        let position_maximum_metres =
            scalar_interval(view, instance, ScalarFact::AxisPositionMaximumMetres)?;
        let usable_position_minimum_metres = position_minimum_metres.upper.clone();
        let usable_position_maximum_metres = position_maximum_metres.lower.clone();
        if usable_position_minimum_metres >= usable_position_maximum_metres {
            return Err(MachineProfileError::EmptyUsableTravel { instance });
        }

        let configured_velocity_limit_metres_per_second =
            positive_scalar_interval(view, instance, ScalarFact::AxisVelocityLimitMetresPerSecond)?;
        let pulse_period_cycles = step
            .minimum_active_cycles
            .checked_add(step.minimum_inactive_cycles)
            .ok_or(MachineProfileError::PulseTimingOverflow { instance })?;
        let cycle_limited_frequency_hz =
            Rational::from(timer_ticks_per_second) / Rational::from(pulse_period_cycles);
        let effective_step_frequency_hz = minimum_rational(
            Rational::from(step.maximum_frequency_hz),
            cycle_limited_frequency_hz,
        );
        let step_rate_velocity_limit_metres_per_second =
            &effective_step_frequency_hz / &command_density_steps_per_metre.upper;
        let effective_velocity_limit_metres_per_second = minimum_rational(
            configured_velocity_limit_metres_per_second.lower.clone(),
            step_rate_velocity_limit_metres_per_second.clone(),
        );

        let acceleration_limit_metres_per_second_squared = positive_scalar_interval(
            view,
            instance,
            ScalarFact::AxisAccelerationLimitMetresPerSecondSquared,
        )?;
        let effective_acceleration_limit_metres_per_second_squared =
            acceleration_limit_metres_per_second_squared.lower.clone();
        let jerk_limit_metres_per_second_cubed = positive_scalar_interval(
            view,
            instance,
            ScalarFact::AxisJerkLimitMetresPerSecondCubed,
        )?;
        let effective_jerk_limit_metres_per_second_cubed =
            jerk_limit_metres_per_second_cubed.lower.clone();
        let following_error_metres =
            scalar_interval(view, instance, ScalarFact::AxisFollowingErrorMetres)?;
        let maximum_following_error_metres = following_error_metres.upper.clone();

        Ok(Self {
            instance,
            step,
            direction,
            driver_control,
            driver_control_action,
            full_steps_per_turn,
            microsteps,
            motor_turns_per_output_turn,
            travel_metres_per_output_turn,
            calibration_scale,
            command_density_steps_per_metre,
            command_density_steps_per_millimetre,
            position_minimum_metres,
            position_maximum_metres,
            usable_position_minimum_metres,
            usable_position_maximum_metres,
            configured_velocity_limit_metres_per_second,
            effective_step_frequency_hz,
            step_rate_velocity_limit_metres_per_second,
            effective_velocity_limit_metres_per_second,
            acceleration_limit_metres_per_second_squared,
            effective_acceleration_limit_metres_per_second_squared,
            jerk_limit_metres_per_second_cubed,
            effective_jerk_limit_metres_per_second_cubed,
            following_error_metres,
            maximum_following_error_metres,
        })
    }

    /// Logical dense axis instance.
    pub const fn instance(&self) -> u16 {
        self.instance
    }

    /// Validated step-output resource and pulse/rate bounds.
    pub const fn step_binding(&self) -> ResourceBinding {
        self.step
    }

    /// Validated direction resource and setup/hold bounds.
    pub const fn direction_binding(&self) -> ResourceBinding {
        self.direction
    }

    /// Validated driver-control resource and setup/hold bounds.
    pub const fn driver_control_binding(&self) -> ResourceBinding {
        self.driver_control
    }

    /// Whether the driver-control binding asserts enable or disable.
    pub const fn driver_control_action(&self) -> AxisDriverControl {
        self.driver_control_action
    }

    /// Motor full steps per motor turn.
    pub const fn full_steps_per_turn(&self) -> &ExactInterval {
        &self.full_steps_per_turn
    }

    /// Configured microsteps per full step.
    pub const fn microsteps(&self) -> &ExactInterval {
        &self.microsteps
    }

    /// Motor turns per output turn.
    pub const fn motor_turns_per_output_turn(&self) -> &ExactInterval {
        &self.motor_turns_per_output_turn
    }

    /// Linear travel per output turn.
    pub const fn travel_metres_per_output_turn(&self) -> &ExactInterval {
        &self.travel_metres_per_output_turn
    }

    /// Dimensionless command-density calibration multiplier.
    pub const fn calibration_scale(&self) -> &ExactInterval {
        &self.calibration_scale
    }

    /// Derived commanded steps per metre, including all source uncertainty.
    pub const fn command_density_steps_per_metre(&self) -> &ExactInterval {
        &self.command_density_steps_per_metre
    }

    /// Derived commanded steps per millimetre, including all source uncertainty.
    pub const fn command_density_steps_per_millimetre(&self) -> &ExactInterval {
        &self.command_density_steps_per_millimetre
    }

    /// Configured lower travel fact and its uncertainty interval.
    pub const fn position_minimum_metres(&self) -> &ExactInterval {
        &self.position_minimum_metres
    }

    /// Configured upper travel fact and its uncertainty interval.
    pub const fn position_maximum_metres(&self) -> &ExactInterval {
        &self.position_maximum_metres
    }

    /// Lowest command position guaranteed inside every admitted travel interval.
    pub const fn usable_position_minimum_metres(&self) -> &Rational {
        &self.usable_position_minimum_metres
    }

    /// Highest command position guaranteed inside every admitted travel interval.
    pub const fn usable_position_maximum_metres(&self) -> &Rational {
        &self.usable_position_maximum_metres
    }

    /// Configured velocity limit and physical uncertainty interval.
    pub const fn configured_velocity_limit_metres_per_second(&self) -> &ExactInterval {
        &self.configured_velocity_limit_metres_per_second
    }

    /// Conservative velocity ceiling imposed solely by maximum step frequency.
    pub const fn effective_step_frequency_hz(&self) -> &Rational {
        &self.effective_step_frequency_hz
    }

    /// Conservative velocity ceiling imposed by maximum step frequency and pulse timing.
    pub const fn step_rate_velocity_limit_metres_per_second(&self) -> &Rational {
        &self.step_rate_velocity_limit_metres_per_second
    }

    /// Lesser of the uncertain configured limit and the step-frequency limit.
    pub const fn effective_velocity_limit_metres_per_second(&self) -> &Rational {
        &self.effective_velocity_limit_metres_per_second
    }

    /// Configured acceleration limit and physical uncertainty interval.
    pub const fn acceleration_limit_metres_per_second_squared(&self) -> &ExactInterval {
        &self.acceleration_limit_metres_per_second_squared
    }

    /// Conservative acceleration limit used by scheduling.
    pub const fn effective_acceleration_limit_metres_per_second_squared(&self) -> &Rational {
        &self.effective_acceleration_limit_metres_per_second_squared
    }

    /// Configured jerk limit and physical uncertainty interval.
    pub const fn jerk_limit_metres_per_second_cubed(&self) -> &ExactInterval {
        &self.jerk_limit_metres_per_second_cubed
    }

    /// Conservative jerk limit used by scheduling.
    pub const fn effective_jerk_limit_metres_per_second_cubed(&self) -> &Rational {
        &self.effective_jerk_limit_metres_per_second_cubed
    }

    /// Configured following-error fact and uncertainty interval.
    pub const fn following_error_metres(&self) -> &ExactInterval {
        &self.following_error_metres
    }

    /// Conservative physical following-error bound.
    pub const fn maximum_following_error_metres(&self) -> &Rational {
        &self.maximum_following_error_metres
    }
}

/// Two-axis exact machine profile consumed by the current canonical compiler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineDynamicsProfile2 {
    identity: ConfigurationIdentity,
    timer_ticks_per_second: u64,
    output_quantum_cycles: u32,
    axes: [StepperAxisMachineProfile; 2],
}

impl MachineDynamicsProfile2 {
    /// Derives a dense two-axis stepper profile from already validated bytes.
    /// Mixed, sparse, or wider machine layouts are rejected rather than
    /// silently projecting away configured motion axes.
    pub fn from_configuration(view: ConfigurationDocumentView<'_>) -> MachineProfileResult<Self> {
        let identity = view.identity();
        if identity.summary.stepper_axes != 2 || identity.summary.foc_axes != 0 {
            return Err(MachineProfileError::UnsupportedAxisLayout {
                steppers: identity.summary.stepper_axes,
                foc: identity.summary.foc_axes,
            });
        }
        let timer = required_scalar(view, 0, ScalarFact::TimerTickHertz)?;
        let timer_ticks_per_second = u64::try_from(timer.value.numerator).map_err(|_| {
            MachineProfileError::InvalidTimerTick {
                numerator: timer.value.numerator,
                denominator: timer.value.denominator,
            }
        })?;
        if timer_ticks_per_second == 0
            || timer.value.denominator != 1
            || timer.uncertainty.numerator != 0
        {
            return Err(MachineProfileError::InvalidTimerTick {
                numerator: timer.value.numerator,
                denominator: timer.value.denominator,
            });
        }
        let output_quantum = required_scalar(view, 0, ScalarFact::StepperOutputQuantumCycles)?;
        let output_quantum_cycles =
            u32::try_from(output_quantum.value.numerator).map_err(|_| {
                MachineProfileError::InvalidOutputQuantum {
                    numerator: output_quantum.value.numerator,
                    denominator: output_quantum.value.denominator,
                }
            })?;
        if output_quantum_cycles == 0
            || output_quantum.value.denominator != 1
            || output_quantum.uncertainty.numerator != 0
        {
            return Err(MachineProfileError::InvalidOutputQuantum {
                numerator: output_quantum.value.numerator,
                denominator: output_quantum.value.denominator,
            });
        }
        Ok(Self {
            identity,
            timer_ticks_per_second,
            output_quantum_cycles,
            axes: [
                StepperAxisMachineProfile::derive(view, 0, timer_ticks_per_second)?,
                StepperAxisMachineProfile::derive(view, 1, timer_ticks_per_second)?,
            ],
        })
    }

    /// Exact canonical configuration identity behind every derived fact.
    pub const fn configuration_identity(&self) -> ConfigurationIdentity {
        self.identity
    }

    /// Canonical configuration SHA-256.
    pub const fn configuration_digest(&self) -> Digest {
        self.identity.digest
    }

    /// Board capability SHA-256 to which the configuration was validated.
    pub const fn capability_digest(&self) -> Digest {
        self.identity.capability_digest
    }

    /// Exact integer `DeviceCycle` frequency.
    pub const fn timer_ticks_per_second(&self) -> u64 {
        self.timer_ticks_per_second
    }

    /// Smallest addressable output interval in device cycles.
    pub const fn output_quantum_cycles(&self) -> u32 {
        self.output_quantum_cycles
    }

    /// Production step/direction electrical policy derived from the same
    /// canonical records used for CAM. Lateness does not alter ideal event
    /// times, but is retained for byte-identical executor replay.
    pub fn stepper_timing(&self, maximum_lateness_cycles: u32) -> StepperTiming<2> {
        StepperTiming {
            axes: self.axes.each_ref().map(|axis| AxisTiming {
                pulse_high_cycles: axis.step.minimum_active_cycles,
                pulse_low_cycles: axis.step.minimum_inactive_cycles,
                direction_setup_cycles: axis.direction.minimum_active_cycles,
                direction_hold_cycles: axis.direction.minimum_inactive_cycles,
                enable_setup_cycles: axis.driver_control.minimum_active_cycles,
                enable_hold_cycles: axis.driver_control.minimum_inactive_cycles,
                maximum_step_frequency_hz: axis.step.maximum_frequency_hz,
            }),
            device_cycle_hz: self.timer_ticks_per_second,
            output_quantum_cycles: self.output_quantum_cycles,
            maximum_lateness_cycles,
        }
    }

    /// Dense X/Y axis profiles.
    pub const fn axes(&self) -> &[StepperAxisMachineProfile; 2] {
        &self.axes
    }
}

/// Certified decomposition of the first unavoidable two-axis error floor.
#[derive(Clone, Debug, PartialEq)]
pub struct MachineResolutionBudget2 {
    configuration_digest: Digest,
    capability_digest: Digest,
    requested_total_error_mm_exact: Rational,
    source_curve_allocation_mm_exact: Rational,
    controller_interpolation_allocation_mm_exact: Rational,
    requested_total_error_mm: Real,
    source_curve_allocation_mm: Real,
    controller_interpolation_allocation_mm: Real,
    endpoint_quantization_error_mm: Real,
    step_event_tracking_error_mm: Real,
    command_lattice_error_mm: Real,
    calibration_error_mm: Real,
    following_error_mm: Real,
    output_grid_position_error_mm: Real,
    required_total_error_mm: Real,
}

impl MachineResolutionBudget2 {
    /// Computes conservative machine-wide components and proves their sum fits
    /// the requested total. Calibration error is bounded over the complete
    /// usable travel relative to machine coordinate zero; a later per-job
    /// certificate may tighten that extent but may not exceed this envelope.
    pub fn certify(
        profile: &MachineDynamicsProfile2,
        requested_total_error_mm: Rational,
        source_curve_allocation_mm: Rational,
        controller_interpolation_allocation_mm: Rational,
    ) -> MachineProfileResult<Self> {
        if requested_total_error_mm <= Rational::zero()
            || source_curve_allocation_mm < Rational::zero()
            || controller_interpolation_allocation_mm < Rational::zero()
        {
            return Err(MachineProfileError::InvalidErrorBudget);
        }

        let mut endpoint_lattice_axis = Vec::with_capacity(2);
        let mut step_event_axis = Vec::with_capacity(2);
        let mut calibration_axis = Vec::with_capacity(2);
        let mut following_axis = Vec::with_capacity(2);
        let mut velocity_axis_mm = Vec::with_capacity(2);
        for axis in profile.axes() {
            let half_step_mm = Rational::one()
                / (Rational::from(2) * axis.command_density_steps_per_millimetre().lower());
            endpoint_lattice_axis.push(Real::from(half_step_mm));
            step_event_axis.push(Real::from(
                Rational::one() / axis.command_density_steps_per_millimetre().lower(),
            ));

            let density = axis.command_density_steps_per_metre();
            let lower_scale_error = (density.nominal() / density.lower()) - Rational::one();
            let upper_scale_error = Rational::one() - (density.nominal() / density.upper());
            let maximum_scale_error = maximum_rational(lower_scale_error, upper_scale_error);
            let maximum_extent_metres = maximum_rational(
                absolute_rational(axis.usable_position_minimum_metres()),
                absolute_rational(axis.usable_position_maximum_metres()),
            );
            calibration_axis.push(Real::from(
                maximum_extent_metres * Rational::from(1_000) * maximum_scale_error,
            ));
            following_axis.push(Real::from(
                axis.maximum_following_error_metres() * Rational::from(1_000),
            ));
            velocity_axis_mm.push(Real::from(
                axis.effective_velocity_limit_metres_per_second() * Rational::from(1_000),
            ));
        }

        let endpoint_quantization_error_mm = vector_norm2(&endpoint_lattice_axis)?;
        let step_event_tracking_error_mm = vector_norm2(&step_event_axis)?;
        let command_lattice_error_mm =
            endpoint_quantization_error_mm.clone() + step_event_tracking_error_mm.clone();
        let calibration_error_mm = vector_norm2(&calibration_axis)?;
        let following_error_mm = vector_norm2(&following_axis)?;
        let maximum_vector_velocity_mm_per_second = vector_norm2(&velocity_axis_mm)?;
        let output_quantum_seconds = (Real::from(profile.output_quantum_cycles())
            / Real::from(profile.timer_ticks_per_second()))?;
        let output_grid_position_error_mm =
            maximum_vector_velocity_mm_per_second * output_quantum_seconds;
        let requested_total_error_mm_exact = requested_total_error_mm;
        let source_curve_allocation_mm_exact = source_curve_allocation_mm;
        let controller_interpolation_allocation_mm_exact = controller_interpolation_allocation_mm;
        let requested_total_error_mm = Real::from(requested_total_error_mm_exact.clone());
        let source_curve_allocation_mm = Real::from(source_curve_allocation_mm_exact.clone());
        let controller_interpolation_allocation_mm =
            Real::from(controller_interpolation_allocation_mm_exact.clone());
        let required_total_error_mm = source_curve_allocation_mm.clone()
            + controller_interpolation_allocation_mm.clone()
            + command_lattice_error_mm.clone()
            + calibration_error_mm.clone()
            + following_error_mm.clone()
            + output_grid_position_error_mm.clone();
        match compare_reals(
            &required_total_error_mm,
            &requested_total_error_mm,
            PredicatePolicy::STRICT,
        )
        .value()
        {
            Some(Ordering::Less | Ordering::Equal) => Ok(Self {
                configuration_digest: profile.configuration_digest(),
                capability_digest: profile.capability_digest(),
                requested_total_error_mm_exact,
                source_curve_allocation_mm_exact,
                controller_interpolation_allocation_mm_exact,
                requested_total_error_mm,
                source_curve_allocation_mm,
                controller_interpolation_allocation_mm,
                endpoint_quantization_error_mm,
                step_event_tracking_error_mm,
                command_lattice_error_mm,
                calibration_error_mm,
                following_error_mm,
                output_grid_position_error_mm,
                required_total_error_mm,
            }),
            Some(Ordering::Greater) => Err(MachineProfileError::ErrorBudgetExceeded),
            None => Err(MachineProfileError::ErrorBudgetPredicateUnresolved),
        }
    }

    /// Canonical configuration identity for which this budget was certified.
    pub const fn configuration_digest(&self) -> Digest {
        self.configuration_digest
    }

    /// Immutable board capability identity for which this budget was certified.
    pub const fn capability_digest(&self) -> Digest {
        self.capability_digest
    }

    /// Requested total as the caller's exact rational input.
    pub const fn requested_total_error_mm_exact(&self) -> &Rational {
        &self.requested_total_error_mm_exact
    }

    /// Source allocation as the caller's exact rational input.
    pub const fn source_curve_allocation_mm_exact(&self) -> &Rational {
        &self.source_curve_allocation_mm_exact
    }

    /// Controller interpolation allocation as the caller's exact rational input.
    pub const fn controller_interpolation_allocation_mm_exact(&self) -> &Rational {
        &self.controller_interpolation_allocation_mm_exact
    }

    /// Requested total positional error in millimetres.
    pub const fn requested_total_error_mm(&self) -> &Real {
        &self.requested_total_error_mm
    }

    /// Caller-owned source-curve approximation allocation.
    pub const fn source_curve_allocation_mm(&self) -> &Real {
        &self.source_curve_allocation_mm
    }

    /// Caller-owned V1 constant-segment interpolation allocation.
    pub const fn controller_interpolation_allocation_mm(&self) -> &Real {
        &self.controller_interpolation_allocation_mm
    }

    /// Worst-case Euclidean nearest-endpoint quantization component.
    pub const fn endpoint_quantization_error_mm(&self) -> &Real {
        &self.endpoint_quantization_error_mm
    }

    /// Worst-case Euclidean DDA step-event tracking component.
    pub const fn step_event_tracking_error_mm(&self) -> &Real {
        &self.step_event_tracking_error_mm
    }

    /// Sum of endpoint quantization and within-segment DDA tracking components.
    pub const fn command_lattice_error_mm(&self) -> &Real {
        &self.command_lattice_error_mm
    }

    /// Worst-case full-travel density/calibration component.
    pub const fn calibration_error_mm(&self) -> &Real {
        &self.calibration_error_mm
    }

    /// Worst-case Euclidean configured following-error component.
    pub const fn following_error_mm(&self) -> &Real {
        &self.following_error_mm
    }

    /// One-output-quantum positional component at maximum vector velocity.
    ///
    /// Timer lowering ceilings each factor-scaled interval, so its retained
    /// grid-only padding is strictly below this duration. Intentional exact
    /// schedule dilation is reported separately and is not misclassified as a
    /// spatial approximation error.
    pub const fn output_grid_position_error_mm(&self) -> &Real {
        &self.output_grid_position_error_mm
    }

    /// Sum proven no greater than the requested total.
    pub const fn required_total_error_mm(&self) -> &Real {
        &self.required_total_error_mm
    }
}

fn vector_norm2(components: &[Real]) -> Result<Real, Problem> {
    let mut squared = Real::zero();
    for component in components {
        squared += component * component;
    }
    squared.sqrt()
}

fn required_binding(
    view: ConfigurationDocumentView<'_>,
    instance: u16,
    role: BindingRole,
) -> MachineProfileResult<ResourceBinding> {
    view.binding(instance, role)?
        .ok_or(MachineProfileError::MissingBinding { instance, role })
}

fn required_scalar(
    view: ConfigurationDocumentView<'_>,
    instance: u16,
    fact: ScalarFact,
) -> MachineProfileResult<ExactScalar> {
    view.scalar(instance, fact)?
        .ok_or(MachineProfileError::MissingScalar { instance, fact })
}

fn scalar_interval(
    view: ConfigurationDocumentView<'_>,
    instance: u16,
    fact: ScalarFact,
) -> MachineProfileResult<ExactInterval> {
    ExactInterval::from_scalar(required_scalar(view, instance, fact)?)
}

fn positive_scalar_interval(
    view: ConfigurationDocumentView<'_>,
    instance: u16,
    fact: ScalarFact,
) -> MachineProfileResult<ExactInterval> {
    scalar_interval(view, instance, fact)?.require_positive_lower(instance, fact)
}

fn convert_rational(value: alumina_config::Rational) -> MachineProfileResult<Rational> {
    Rational::fraction(value.numerator, value.denominator).map_err(MachineProfileError::Arithmetic)
}

fn minimum_rational(left: Rational, right: Rational) -> Rational {
    if left <= right { left } else { right }
}

fn maximum_rational(left: Rational, right: Rational) -> Rational {
    if left >= right { left } else { right }
}

fn absolute_rational(value: &Rational) -> Rational {
    if value.is_negative() {
        -value
    } else {
        value.clone()
    }
}

/// Failure to derive or certify a canonical machine profile.
#[derive(Debug)]
pub enum MachineProfileError {
    /// Canonical configuration inspection unexpectedly failed.
    Configuration(ConfigurationError),
    /// Hyper exact arithmetic rejected an operation.
    Arithmetic(Problem),
    /// The current compiler requires exactly two dense stepper axes and no FOC axis.
    UnsupportedAxisLayout {
        /// Configured stepper-axis count.
        steppers: u8,
        /// Configured FOC-axis count.
        foc: u8,
    },
    /// A required exact fact was absent.
    MissingScalar {
        /// Logical instance.
        instance: u16,
        /// Required fact.
        fact: ScalarFact,
    },
    /// A required resource binding was absent.
    MissingBinding {
        /// Logical instance.
        instance: u16,
        /// Required binding role.
        role: BindingRole,
    },
    /// The axis did not have exactly one enable/disable control semantic.
    InvalidDriverControl {
        /// Logical axis instance.
        instance: u16,
    },
    /// Physical uncertainty reached or crossed zero for a divisive or limit fact.
    NonPositiveLowerBound {
        /// Logical fact instance.
        instance: u16,
        /// Fact whose lower endpoint was not positive.
        fact: ScalarFact,
    },
    /// Conservative position-bound intersection was empty.
    EmptyUsableTravel {
        /// Logical axis instance.
        instance: u16,
    },
    /// Pulse-high plus pulse-low timing overflowed.
    PulseTimingOverflow {
        /// Logical axis instance.
        instance: u16,
    },
    /// The global time-base fact was not a positive exact integer.
    InvalidTimerTick {
        /// Encoded numerator.
        numerator: i64,
        /// Encoded denominator.
        denominator: u64,
    },
    /// The global stepper output quantum was not a positive exact `u32`.
    InvalidOutputQuantum {
        /// Encoded numerator.
        numerator: i64,
        /// Encoded denominator.
        denominator: u64,
    },
    /// Requested or source error allocation had an invalid sign.
    InvalidErrorBudget,
    /// Unavoidable components exceeded the requested total.
    ErrorBudgetExceeded,
    /// Hyperlimit could not decide the final exact budget predicate.
    ErrorBudgetPredicateUnresolved,
}

impl fmt::Display for MachineProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(source) => {
                write!(formatter, "canonical configuration inspection failed: {source:?}")
            }
            Self::Arithmetic(source) => write!(formatter, "exact machine arithmetic failed: {source}"),
            Self::UnsupportedAxisLayout { steppers, foc } => write!(
                formatter,
                "the two-axis compiler cannot consume {steppers} stepper and {foc} FOC axes"
            ),
            Self::MissingScalar { instance, fact } => {
                write!(formatter, "axis {instance} is missing exact fact {fact:?}")
            }
            Self::MissingBinding { instance, role } => {
                write!(formatter, "axis {instance} is missing binding {role:?}")
            }
            Self::InvalidDriverControl { instance } => write!(
                formatter,
                "axis {instance} does not have exactly one enable/disable binding"
            ),
            Self::NonPositiveLowerBound { instance, fact } => write!(
                formatter,
                "axis {instance} fact {fact:?} has a nonpositive uncertainty lower bound"
            ),
            Self::EmptyUsableTravel { instance } => write!(
                formatter,
                "axis {instance} has no travel common to all admitted uncertainty"
            ),
            Self::PulseTimingOverflow { instance } => {
                write!(formatter, "axis {instance} pulse timing overflowed")
            }
            Self::InvalidTimerTick {
                numerator,
                denominator,
            } => write!(
                formatter,
                "timer tick frequency {numerator}/{denominator} is not a positive exact integer"
            ),
            Self::InvalidOutputQuantum {
                numerator,
                denominator,
            } => write!(
                formatter,
                "stepper output quantum {numerator}/{denominator} is not a positive exact u32"
            ),
            Self::InvalidErrorBudget => formatter.write_str("machine error budget is invalid"),
            Self::ErrorBudgetExceeded => formatter.write_str(
                "machine resolution, calibration, following, timing, and source allocations exceed the requested error",
            ),
            Self::ErrorBudgetPredicateUnresolved => {
                formatter.write_str("the exact machine error-budget predicate remained unresolved")
            }
        }
    }
}

impl StdError for MachineProfileError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Arithmetic(source) => Some(source),
            _ => None,
        }
    }
}

impl From<ConfigurationError> for MachineProfileError {
    fn from(value: ConfigurationError) -> Self {
        Self::Configuration(value)
    }
}

impl From<Problem> for MachineProfileError {
    fn from(value: Problem) -> Self {
        Self::Arithmetic(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::CanonicalStep;
    use crate::compiler::{
        MachineCompileError, MotionCompilePolicy2, compile_certified_chord_program,
    };
    use crate::direct_motion::{
        DirectFiniteDifferencePolicy2, lower_certified_schedule_to_direct_finite_difference,
    };
    use crate::global_job::{
        SharedGlobalJobCompilePolicy2, SharedScheduledJobParticipant2,
        compile_shared_scheduled_global_job,
    };
    use crate::motion_schedule::{
        CanonicalScheduledProgram2, CertifiedJerkSchedule2, MotionScheduleError,
        ScheduledLoweringLimits, SharedTimerCandidateOutcome2, SharedTimerParticipant2,
        TimerDilationPolicy, TravelBoundary, certify_jerk_schedule, lower_certified_schedule_to_v1,
        select_shared_timer_lattice_schedule,
    };
    use crate::partition::{
        MachinePartitionError, MachinePartitionPolicy2, package_canonical_direct_program,
        package_canonical_scheduled_program, package_shared_retimed_scheduled_program,
    };
    use crate::schedule_evidence::{
        ScheduleEvidenceError, build_canonical_schedule_evidence,
        replay_canonical_schedule_evidence, verify_canonical_schedule_evidence_bytes,
    };
    use crate::shared_timing_evidence::{
        SharedTimingEvidenceError, SharedTimingEvidenceParticipant2, build_shared_timing_evidence,
        replay_shared_timing_evidence, verify_shared_timing_evidence_bytes,
    };
    use crate::toolpath::{
        MetricPathApproximationLimits2, representative_curve_path, representative_metric_path,
    };
    use alumina_board::{OwnerDomain, ResourceId};
    use alumina_config::{
        BindingFlags, ConfigurationFlags, ConfigurationHeader, ConfigurationRecord, ExactScalar,
        FactEvidence, Rational as ConfigurationRational, SignalPolarity,
    };
    use alumina_job::{JobNetworkPolicy, MachineJobGlobalFacts};
    use alumina_machine_ir::{
        BlockValidationLimits, EXECUTION_BLOCK_BYTES, StreamTick, ValidationLimits,
    };
    use alumina_motion::MotionError;
    use alumina_protocol::DeviceId;
    use alumina_sim::motion::{
        CachedFiniteDifferenceReplayError, CachedStepperReplayError,
        replay_cached_finite_difference_partition, replay_cached_stepper_partition,
    };
    use alumina_storage::{CacheLimits, UploadId, sha256};
    use hypercurve::{
        CircularArc2, Curve2, CurveGeometry2, CurvePath2, LineSeg2, Point2 as CurvePoint2,
    };

    fn wire_rational(numerator: i64, denominator: u64) -> ConfigurationRational {
        ConfigurationRational::new(numerator, denominator).unwrap()
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
    ) -> ConfigurationRecord {
        ConfigurationRecord::Scalar(ExactScalar {
            instance,
            fact,
            value: wire_rational(numerator, denominator),
            uncertainty: wire_rational(uncertainty_numerator, uncertainty_denominator),
            evidence: FactEvidence::Measured,
        })
    }

    fn axis_scalars(instance: u16) -> Vec<ConfigurationRecord> {
        vec![
            scalar(instance, ScalarFact::AxisFullStepsPerTurn, 200, 1, 0, 1),
            scalar(instance, ScalarFact::AxisMicrosteps, 16, 1, 0, 1),
            scalar(
                instance,
                ScalarFact::AxisMotorTurnsPerOutputTurn,
                1,
                1,
                0,
                1,
            ),
            scalar(
                instance,
                ScalarFact::AxisTravelMetresPerOutputTurn,
                1,
                500,
                0,
                1,
            ),
            scalar(
                instance,
                ScalarFact::AxisCalibrationScale,
                1,
                1,
                1,
                1_000_000,
            ),
            scalar(instance, ScalarFact::AxisPositionMinimumMetres, 0, 1, 0, 1),
            scalar(instance, ScalarFact::AxisPositionMaximumMetres, 3, 10, 0, 1),
            scalar(
                instance,
                ScalarFact::AxisVelocityLimitMetresPerSecond,
                1,
                20,
                1,
                1_000,
            ),
            scalar(
                instance,
                ScalarFact::AxisAccelerationLimitMetresPerSecondSquared,
                1,
                2,
                1,
                100,
            ),
            scalar(
                instance,
                ScalarFact::AxisJerkLimitMetresPerSecondCubed,
                5,
                1,
                1,
                10,
            ),
            scalar(
                instance,
                ScalarFact::AxisFollowingErrorMetres,
                1,
                100_000,
                1,
                500_000,
            ),
        ]
    }

    fn machine_records() -> Vec<ConfigurationRecord> {
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
        records.extend(axis_scalars(0));
        records.push(scalar(0, ScalarFact::TimerTickHertz, 1_000_000, 1, 0, 1));
        records.push(scalar(
            0,
            ScalarFact::StepperOutputQuantumCycles,
            1,
            1,
            0,
            1,
        ));
        records.extend(axis_scalars(1));
        records.sort_by_key(|record| record.canonical_order_key());
        records
    }

    fn document(records: &[ConfigurationRecord]) -> (Vec<u8>, Digest) {
        let realtime_record_count = records
            .iter()
            .filter(|record| record.realtime_relevant())
            .count();
        let header = ConfigurationHeader {
            capability_digest: board_mks_tinybee::CAPABILITY_DIGEST,
            record_count: u16::try_from(records.len()).unwrap(),
            realtime_record_count: u16::try_from(realtime_record_count).unwrap(),
            flags: ConfigurationFlags(ConfigurationFlags::MOTION),
        };
        let mut bytes = Vec::from(header.encode().unwrap());
        for record in records {
            bytes.extend_from_slice(&record.encode().unwrap());
        }
        let digest = sha256(&bytes).digest;
        (bytes, digest)
    }

    fn profile_from(
        records: &[ConfigurationRecord],
    ) -> MachineProfileResult<MachineDynamicsProfile2> {
        let (bytes, digest) = document(records);
        let view =
            ConfigurationDocumentView::decode::<32>(&board_mks_tinybee::PACKAGE, &bytes, digest)
                .unwrap();
        MachineDynamicsProfile2::from_configuration(view)
    }

    fn lower_diagonal_g1(
        records: &[ConfigurationRecord],
    ) -> (
        MachineDynamicsProfile2,
        CertifiedJerkSchedule2,
        CanonicalScheduledProgram2,
    ) {
        let profile = profile_from(records).unwrap();
        let source = CurvePath2::try_new(vec![
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(0, 0),
                    CurvePoint2::from_values(3, 4),
                )
                .unwrap(),
            )),
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(3, 4),
                    CurvePoint2::from_values(6, 8),
                )
                .unwrap(),
            )),
        ])
        .unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();
        let lowered = lower_certified_schedule_to_v1(
            &schedule,
            &profile,
            &budget,
            Rational::fraction(1, 1_000).unwrap(),
            ScheduledLoweringLimits::INTERACTIVE,
        )
        .unwrap();
        (profile, schedule, lowered)
    }

    #[test]
    fn canonical_configuration_derives_exact_conservative_machine_limits() {
        let profile = profile_from(&machine_records()).unwrap();
        assert_eq!(profile.timer_ticks_per_second(), 1_000_000);
        assert_eq!(profile.output_quantum_cycles(), 1);
        assert_eq!(
            profile.capability_digest(),
            board_mks_tinybee::CAPABILITY_DIGEST
        );
        let x = &profile.axes()[0];
        assert_eq!(x.instance(), 0);
        assert_eq!(x.driver_control_action(), AxisDriverControl::Disable);
        assert_eq!(
            x.command_density_steps_per_metre().nominal(),
            &Rational::from(1_600_000)
        );
        assert_eq!(
            x.command_density_steps_per_millimetre().nominal(),
            &Rational::from(1_600)
        );
        assert_eq!(
            x.command_density_steps_per_metre().lower(),
            &(Rational::from(1_600_000) * Rational::fraction(999_999, 1_000_000).unwrap())
        );
        assert_eq!(
            x.effective_step_frequency_hz(),
            &Rational::fraction(31_250, 3).unwrap()
        );
        assert_eq!(
            x.effective_velocity_limit_metres_per_second(),
            &(&Rational::fraction(31_250, 3).unwrap()
                / x.command_density_steps_per_metre().upper())
        );
        assert_eq!(
            x.effective_acceleration_limit_metres_per_second_squared(),
            &Rational::fraction(49, 100).unwrap()
        );
        assert_eq!(
            x.effective_jerk_limit_metres_per_second_cubed(),
            &Rational::fraction(49, 10).unwrap()
        );
        assert_eq!(
            x.maximum_following_error_metres(),
            &Rational::fraction(3, 250_000).unwrap()
        );
    }

    #[test]
    fn machine_resolution_budget_accounts_for_every_physical_floor() {
        let profile = profile_from(&machine_records()).unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::fraction(1, 100).unwrap(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        for component in [
            budget.command_lattice_error_mm(),
            budget.calibration_error_mm(),
            budget.following_error_mm(),
            budget.output_grid_position_error_mm(),
        ] {
            assert!(matches!(
                compare_reals(component, &Real::zero(), PredicatePolicy::STRICT).value(),
                Some(Ordering::Greater)
            ));
        }
        assert!(matches!(
            compare_reals(
                budget.required_total_error_mm(),
                budget.requested_total_error_mm(),
                PredicatePolicy::STRICT,
            )
            .value(),
            Some(Ordering::Less | Ordering::Equal)
        ));
        assert!(matches!(
            MachineResolutionBudget2::certify(
                &profile,
                Rational::fraction(1, 1_000).unwrap(),
                Rational::zero(),
                Rational::zero(),
            ),
            Err(MachineProfileError::ErrorBudgetExceeded)
        ));
    }

    #[test]
    fn compiler_policy_is_bound_to_configuration_limits_and_resolution_evidence() {
        let profile = profile_from(&machine_records()).unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::fraction(1, 100).unwrap(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let policy = MotionCompilePolicy2::from_machine_profile(
            &profile,
            &budget,
            Rational::from(5),
            Rational::fraction(1, 1_024).unwrap(),
            24,
        )
        .unwrap();
        assert_eq!(
            policy.machine_configuration_digest(),
            Some(profile.configuration_digest())
        );
        assert_eq!(
            policy.capability_digest(),
            Some(profile.capability_digest())
        );
        assert_eq!(
            policy.steps_per_millimetre(),
            &[Rational::from(1_600), Rational::from(1_600)]
        );
        assert_eq!(policy.resolution_budget(), Some(&budget));

        let program =
            compile_certified_chord_program(&representative_curve_path().unwrap(), &policy)
                .unwrap();
        assert!(!program.segments().is_empty());
        assert!(matches!(
            MotionCompilePolicy2::from_machine_profile(
                &profile,
                &budget,
                Rational::from(50),
                Rational::fraction(1, 1_024).unwrap(),
                24,
            ),
            Err(MachineCompileError::FeedLimitExceeded { axis: 0 })
        ));
        assert!(matches!(
            MotionCompilePolicy2::from_machine_profile(
                &profile,
                &budget,
                Rational::from(10),
                Rational::fraction(1, 50).unwrap(),
                24,
            ),
            Err(MachineCompileError::SourceErrorBudgetExceeded)
        ));
    }

    #[test]
    fn retained_line_arc_path_gets_exact_stop_lookahead_and_four_phase_jerk_replay() {
        let profile = profile_from(&machine_records()).unwrap();
        let source = representative_metric_path().unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();

        assert_eq!(
            schedule.configuration_digest(),
            profile.configuration_digest()
        );
        assert_eq!(schedule.route().len(), 2);
        assert_eq!(schedule.phases().len(), 2);
        assert!(schedule.phases().iter().all(|phases| phases.len() == 4));
        assert_eq!(schedule.lookahead().corner_feeds, vec![Real::zero()]);
        assert_eq!(schedule.lookahead().corner_radii, vec![Real::zero()]);
        assert!(schedule.lookahead_plan().all_satisfied());
        assert_eq!(
            schedule
                .acceleration_lookahead_plan()
                .effective_node_feed_limits,
            vec![Real::zero(); schedule.route().len() + 1]
        );
        assert_eq!(
            schedule.acceleration_lookahead_plan().forward_node_feeds,
            vec![Real::zero(); schedule.route().len() + 1]
        );
        assert!(schedule.lookahead_report().all_satisfied());
        assert!(schedule.jerk_report().all_satisfied());
        assert!(schedule.limits().affine_axis_projection().is_none());
        assert_eq!(
            schedule.travel_envelope().source_minimum_mm(),
            &[Real::zero(), Real::zero()]
        );
        assert_eq!(
            schedule.travel_envelope().source_maximum_mm(),
            &[Real::from(8), Real::from(2)]
        );
        assert_eq!(
            schedule.travel_envelope().usable_minimum_mm(),
            &[Real::zero(), Real::zero()]
        );
        assert_eq!(
            schedule.travel_envelope().usable_maximum_mm(),
            &[Real::from(300), Real::from(300)]
        );
        assert_eq!(
            schedule.total_path_length_mm(),
            &(Real::from(4) + Real::from(2) * Real::pi())
        );
        for phases in schedule.phases() {
            assert_eq!(phases[0].ramp.start_feed, Real::zero());
            assert_eq!(phases[0].ramp.start_acceleration, Real::zero());
            assert_eq!(phases[3].ramp.end_feed, Real::zero());
            assert_eq!(phases[3].ramp.end_acceleration, Real::zero());
        }
        assert!(matches!(
            lower_certified_schedule_to_direct_finite_difference(
                &schedule,
                &profile,
                &budget,
                DirectFiniteDifferencePolicy2::interactive(Rational::fraction(1, 1_000).unwrap())
                    .unwrap(),
            ),
            Err(
                crate::direct_motion::DirectMotionError::UnsupportedRouteElement {
                    element_index: 1
                }
            )
        ));

        let lowered = lower_certified_schedule_to_v1(
            &schedule,
            &profile,
            &budget,
            Rational::fraction(1, 1_000).unwrap(),
            ScheduledLoweringLimits::INTERACTIVE,
        )
        .unwrap();
        assert_eq!(
            lowered.configuration_digest(),
            profile.configuration_digest()
        );
        assert_eq!(lowered.points().len(), lowered.segments().len() + 1);
        assert_eq!(
            lowered.points().first().unwrap().steps(),
            [CanonicalStep::new(0); 2]
        );
        assert_eq!(
            lowered.points().last().unwrap().steps(),
            [CanonicalStep::new(12_800), CanonicalStep::new(0)]
        );
        assert!(
            lowered
                .points()
                .windows(2)
                .all(|pair| pair[0].tick() < pair[1].tick())
        );
        assert!(
            lowered
                .segments()
                .windows(2)
                .all(|pair| pair[0].end_tick == pair[1].start_tick)
        );
        assert!(matches!(
            compare_reals(
                lowered.evidence().maximum_chord_interpolation_error_mm(),
                lowered.evidence().requested_interpolation_error_mm(),
                PredicatePolicy::STRICT,
            )
            .value(),
            Some(Ordering::Less | Ordering::Equal)
        ));
        assert!(matches!(
            lower_certified_schedule_to_v1(
                &schedule,
                &profile,
                &budget,
                Rational::fraction(1, 50).unwrap(),
                ScheduledLoweringLimits::INTERACTIVE,
            ),
            Err(crate::motion_schedule::MotionScheduleError::InterpolationAllocationExceeded)
        ));

        let partition_policy = MachinePartitionPolicy2::try_new(
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
                maximum_chunk_bytes: 1_024,
                maximum_chunks: 10_000,
            },
        )
        .unwrap();
        let partition = package_canonical_scheduled_program(&lowered, partition_policy).unwrap();
        assert_eq!(partition.local_timer_hz(), 1_000_000);
        assert_eq!(partition.initial_position(), [0, 0]);
        assert_eq!(partition.final_position(), [12_800, 0]);
        assert_eq!(
            partition.terminal_progress().end_tick,
            lowered.executor_preflight().end_tick
        );
        let replay = replay_cached_stepper_partition::<2>(
            partition.bytes(),
            partition.job_descriptor(0x8877).unwrap(),
            profile.stepper_timing(0),
        )
        .unwrap();
        assert_eq!(replay.block_count, partition.block_count());
        assert_eq!(replay.segment_count as usize, lowered.segments().len());
        assert_eq!(replay.terminal_position, [12_800, 0]);
        assert_eq!(replay.terminal_tick, lowered.executor_preflight().end_tick);
        assert_eq!(
            replay.terminal_block_digest,
            partition.terminal_progress().block_digest
        );
        let mut corrupt_partition = partition.bytes().to_vec();
        corrupt_partition[EXECUTION_BLOCK_BYTES - 1] ^= 1;
        assert_eq!(
            replay_cached_stepper_partition::<2>(
                &corrupt_partition,
                partition.job_descriptor(0x8877).unwrap(),
                profile.stepper_timing(0),
            ),
            Err(CachedStepperReplayError::PartitionIdentity)
        );
        let evidence = build_canonical_schedule_evidence(&schedule, &lowered, &partition).unwrap();
        let rebuilt = build_canonical_schedule_evidence(&schedule, &lowered, &partition).unwrap();
        assert_eq!(evidence, rebuilt);
        assert!(!evidence.digest().is_zero());
        assert!(!evidence.source_digest().is_zero());
        assert!(!evidence.metric_path_digest().is_zero());
        assert!(!evidence.source_approximation_digest().is_zero());
        assert!(!evidence.planner_transcript_digest().is_zero());
        assert!(evidence.planner_transcript_byte_len() > 0);
        assert!(!evidence.lowering_transcript_digest().is_zero());
        assert!(evidence.lowering_transcript_byte_len() > 0);
        assert_eq!(&evidence.encoded()[..8], b"ALMEVD03");
        replay_canonical_schedule_evidence(&evidence, &schedule, &lowered, &partition).unwrap();

        // Exact-real approximation caches are accelerators, not transcript
        // state. Refining representative symbolic planner and lowering values
        // must leave the canonical evidence byte-for-byte unchanged.
        assert!(
            schedule
                .total_path_length_mm()
                .certified_dyadic_interval(-128)
                .is_some()
        );
        assert!(
            schedule
                .total_traversal_time_seconds()
                .certified_dyadic_interval(-128)
                .is_some()
        );
        assert!(
            lowered
                .evidence()
                .maximum_curve_to_canonical_error_mm()
                .certified_dyadic_interval(-128)
                .is_some()
        );
        let cache_refined_evidence =
            build_canonical_schedule_evidence(&schedule, &lowered, &partition).unwrap();
        assert_eq!(cache_refined_evidence, evidence);

        let alternate_schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::try_new(16_385, 21).unwrap(),
        )
        .unwrap();
        let alternate_planner_evidence =
            build_canonical_schedule_evidence(&alternate_schedule, &lowered, &partition).unwrap();
        assert_eq!(
            alternate_planner_evidence.lowering_transcript_digest(),
            evidence.lowering_transcript_digest()
        );
        assert_ne!(
            alternate_planner_evidence.planner_transcript_digest(),
            evidence.planner_transcript_digest()
        );
        assert_ne!(alternate_planner_evidence.digest(), evidence.digest());

        let alternate_timer_limits = ScheduledLoweringLimits::try_new_with_timer_dilation(
            ScheduledLoweringLimits::INTERACTIVE.maximum_points(),
            TimerDilationPolicy::try_new(1_024, 16_384).unwrap(),
        )
        .unwrap();
        let alternate_lowered = lower_certified_schedule_to_v1(
            &schedule,
            &profile,
            &budget,
            Rational::fraction(1, 1_000).unwrap(),
            alternate_timer_limits,
        )
        .unwrap();
        assert_eq!(alternate_lowered.points(), lowered.points());
        assert_eq!(alternate_lowered.segments(), lowered.segments());
        let alternate_partition =
            package_canonical_scheduled_program(&alternate_lowered, partition_policy).unwrap();
        assert_eq!(alternate_partition.bytes(), partition.bytes());
        let alternate_lowering_evidence =
            build_canonical_schedule_evidence(&schedule, &alternate_lowered, &alternate_partition)
                .unwrap();
        assert_eq!(
            alternate_lowering_evidence.planner_transcript_digest(),
            evidence.planner_transcript_digest()
        );
        assert_ne!(
            alternate_lowering_evidence.lowering_transcript_digest(),
            evidence.lowering_transcript_digest()
        );
        assert_ne!(alternate_lowering_evidence.digest(), evidence.digest());

        let mut corrupt_evidence = evidence.encoded().to_vec();
        corrupt_evidence[12] ^= 1;
        assert_eq!(
            verify_canonical_schedule_evidence_bytes(
                &corrupt_evidence,
                evidence.digest(),
                &schedule,
                &lowered,
                &partition,
            ),
            Err(ScheduleEvidenceError::DigestMismatch)
        );

        let wrong_identity = MachinePartitionPolicy2::try_new(
            [0x41; 16],
            Digest([0x99; 32]),
            profile.configuration_digest(),
            partition_policy.block_limits(),
            partition_policy.upload_id(),
            partition_policy.storage_chunk_bytes(),
            partition_policy.cache_limits(),
        )
        .unwrap();
        assert!(matches!(
            package_canonical_scheduled_program(&lowered, wrong_identity),
            Err(MachinePartitionError::ProgramIdentityMismatch)
        ));
    }

    #[test]
    fn stop_to_stop_line_lowers_to_certified_direct_finite_differences() {
        let profile = profile_from(&machine_records()).unwrap();
        let source = CurvePath2::try_new(vec![Curve2::new(CurveGeometry2::Line(
            LineSeg2::try_new(
                CurvePoint2::from_values(0, 0),
                CurvePoint2::from_values(1, 0),
            )
            .unwrap(),
        ))])
        .unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();
        let policy =
            DirectFiniteDifferencePolicy2::interactive(Rational::fraction(1, 1_000).unwrap())
                .unwrap();
        let direct = lower_certified_schedule_to_direct_finite_difference(
            &schedule, &profile, &budget, policy,
        )
        .unwrap();

        assert_eq!(direct.initial_position(), [0, 0]);
        assert_eq!(direct.final_position(), [1_600, 0]);
        assert_eq!(direct.grid_phases().len(), 1);
        assert_eq!(direct.grid_phases()[0].len(), 4);
        assert!(direct.grid_jerk_report().all_satisfied());
        assert!(!direct.records().is_empty());
        assert_eq!(
            direct.executor_preflight().segment_count as usize,
            direct.records().len()
        );
        assert_eq!(
            direct.executor_preflight().update_count,
            direct.evidence().total_update_count()
        );
        assert!(
            direct.evidence().maximum_position_error_mm()
                <= direct.evidence().policy().maximum_position_error_mm()
        );
        assert_eq!(
            direct.evidence().phase_evidence().len(),
            direct.grid_phases().len() * 4
        );
        assert_eq!(
            direct.evidence().record_evidence().len(),
            direct.records().len()
        );
        let mut incoming_error = [Rational::zero(), Rational::zero()];
        for record in direct.evidence().record_evidence() {
            for (axis, expected) in incoming_error.iter_mut().enumerate() {
                assert_eq!(
                    record.axes()[axis].incoming_position_error_steps(),
                    expected
                );
                assert!(record.axes()[axis].terminal_position_error_steps() >= expected);
                *expected = record.axes()[axis].terminal_position_error_steps().clone();
            }
        }
        assert!(direct.records().iter().any(|record| {
            record.update_count < direct.evidence().policy().maximum_updates_per_record()
        }));

        assert!(matches!(
            DirectFiniteDifferencePolicy2::try_new(
                0,
                256,
                10_000,
                128,
                Rational::fraction(1, 1_000).unwrap(),
            ),
            Err(crate::direct_motion::DirectMotionError::InvalidPolicy)
        ));
        let record_limited = DirectFiniteDifferencePolicy2::try_new(
            1,
            256,
            10_000,
            128,
            Rational::fraction(1, 1_000).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            lower_certified_schedule_to_direct_finite_difference(
                &schedule,
                &profile,
                &budget,
                record_limited,
            ),
            Err(crate::direct_motion::DirectMotionError::RecordBudgetExceeded { maximum: 1 })
        ));
        let error_limited = DirectFiniteDifferencePolicy2::try_new(
            65_536,
            256,
            10_000,
            128,
            Rational::fraction(1, 1_000_000_000_000).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            lower_certified_schedule_to_direct_finite_difference(
                &schedule,
                &profile,
                &budget,
                error_limited,
            ),
            Err(crate::direct_motion::DirectMotionError::PositionErrorBudgetExceeded { .. })
        ));

        let mut expected_tick = StreamTick(0);
        let mut expected_finite_position = [0_i64; 2];
        for (segment, evidence) in direct
            .records()
            .iter()
            .zip(direct.evidence().record_evidence())
        {
            assert_eq!(segment, &evidence.segment());
            assert_eq!(segment.start_tick, expected_tick);
            assert_eq!(segment.update_period_ticks, profile.output_quantum_cycles());
            for (axis, expected) in expected_finite_position.iter_mut().enumerate() {
                assert_eq!(segment.axes[axis].initial_position, *expected);
                assert_eq!(
                    segment.axes[axis].first_difference,
                    evidence.axes()[axis].first_difference().encoded_q31_32()
                );
                assert_eq!(
                    segment.axes[axis].second_difference,
                    evidence.axes()[axis].second_difference().encoded_q31_32()
                );
                assert_eq!(
                    segment.axes[axis].third_difference,
                    evidence.axes()[axis].third_difference().encoded_q31_32()
                );
                *expected = segment.position_at(axis, segment.update_count).unwrap();
            }
            expected_tick = segment.end_tick;
        }
        assert_eq!(
            expected_finite_position,
            direct.executor_preflight().terminal_finite_position
        );

        let partition_policy = MachinePartitionPolicy2::try_new(
            [0x51; 16],
            profile.capability_digest(),
            profile.configuration_digest(),
            BlockValidationLimits {
                maximum_block_ticks: 10_000_000,
                segment: ValidationLimits {
                    maximum_segment_ticks: 10_000_000,
                    maximum_steps_per_segment: 100_000,
                },
            },
            UploadId(0x2233_4455_6677_8899),
            700,
            CacheLimits {
                maximum_object_bytes: 4 * 1024 * 1024,
                maximum_chunk_bytes: 1_024,
                maximum_chunks: 10_000,
            },
        )
        .unwrap();
        let partition = package_canonical_direct_program(&direct, partition_policy).unwrap();
        assert_eq!(
            partition.execution_kind(),
            alumina_machine_ir::ExecutionKind::FiniteDifference
        );
        assert_eq!(partition.maximum_segments_per_block(), 4);
        assert_eq!(partition.initial_position(), [0, 0]);
        assert_eq!(partition.final_position(), [1_600, 0]);
        assert_eq!(
            partition.terminal_finite_position(),
            Some(direct.executor_preflight().terminal_finite_position)
        );
        assert_eq!(
            partition.finite_difference_update_count(),
            Some(direct.executor_preflight().update_count)
        );
        let descriptor = partition.job_descriptor(0x9988).unwrap();
        assert_eq!(
            descriptor.execution_kind,
            alumina_machine_ir::ExecutionKind::FiniteDifference
        );
        assert_eq!(
            descriptor.maximum_finite_difference_updates,
            partition.maximum_finite_difference_updates()
        );
        let replay = replay_cached_finite_difference_partition::<2>(
            partition.bytes(),
            descriptor,
            profile.stepper_timing(0),
        )
        .unwrap();
        assert_eq!(replay.block_count, partition.block_count());
        assert_eq!(replay.segment_count as usize, direct.records().len());
        assert_eq!(
            replay.update_count,
            direct.executor_preflight().update_count
        );
        assert_eq!(replay.terminal_position, [1_600, 0]);
        assert_eq!(
            replay.terminal_finite_position,
            direct.executor_preflight().terminal_finite_position
        );
        assert_eq!(replay.terminal_tick, direct.executor_preflight().end_tick);
        assert_eq!(
            replay.terminal_block_digest,
            partition.terminal_progress().block_digest
        );
        let mut corrupt = partition.bytes().to_vec();
        corrupt[EXECUTION_BLOCK_BYTES - 1] ^= 1;
        assert_eq!(
            replay_cached_finite_difference_partition::<2>(
                &corrupt,
                descriptor,
                profile.stepper_timing(0),
            ),
            Err(CachedFiniteDifferenceReplayError::PartitionIdentity)
        );

        let evidence = crate::direct_motion_evidence::build_direct_motion_evidence(
            &schedule, &direct, &partition,
        )
        .unwrap();
        let rebuilt = crate::direct_motion_evidence::build_direct_motion_evidence(
            &schedule, &direct, &partition,
        )
        .unwrap();
        assert_eq!(evidence, rebuilt);
        assert!(!evidence.digest().is_zero());
        assert!(!evidence.transcript_digest().is_zero());
        assert_eq!(evidence.encoded().len(), 344);
        assert!(evidence.transcript_byte_len() > evidence.encoded().len() as u64);
        assert_eq!(evidence.record_count() as usize, direct.records().len());
        assert_eq!(
            evidence.update_count(),
            direct.executor_preflight().update_count
        );
        crate::direct_motion_evidence::replay_direct_motion_evidence(
            &evidence, &schedule, &direct, &partition,
        )
        .unwrap();
        let mut corrupt_evidence = evidence.encoded().to_vec();
        corrupt_evidence[0] ^= 1;
        assert!(matches!(
            crate::direct_motion_evidence::verify_direct_motion_evidence_bytes(
                &corrupt_evidence,
                evidence.digest(),
                &schedule,
                &direct,
                &partition,
            ),
            Err(crate::direct_motion_evidence::DirectMotionEvidenceError::DigestMismatch)
        ));
        assert!(matches!(
            crate::direct_motion_evidence::verify_direct_motion_evidence_bytes(
                &corrupt_evidence,
                sha256(&corrupt_evidence).digest,
                &schedule,
                &direct,
                &partition,
            ),
            Err(crate::direct_motion_evidence::DirectMotionEvidenceError::ReplayMismatch)
        ));
    }

    #[test]
    fn exact_g1_split_retains_positive_feed_and_two_phase_transitions() {
        let profile = profile_from(&machine_records()).unwrap();
        let source = CurvePath2::try_new(vec![
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(0, 0),
                    CurvePoint2::from_values(1, 0),
                )
                .unwrap(),
            )),
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(1, 0),
                    CurvePoint2::from_values(2, 0),
                )
                .unwrap(),
            )),
        ])
        .unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();

        assert_eq!(schedule.route().len(), 2);
        assert_eq!(schedule.lookahead().corner_feeds.len(), 1);
        assert_ne!(schedule.lookahead().corner_feeds[0], Real::zero());
        assert_eq!(schedule.lookahead().corner_radii, vec![Real::zero()]);
        assert_eq!(schedule.lookahead_plan().positive_node_components.len(), 1);
        assert!(
            schedule
                .lookahead_plan()
                .span_transitions
                .iter()
                .all(Option::is_some)
        );
        assert!(schedule.phases().iter().all(|phases| phases.len() == 2));
        assert_eq!(schedule.phases()[0][0].ramp.start_feed, Real::zero());
        assert_eq!(
            schedule.phases()[0][1].ramp.end_feed,
            schedule.lookahead().corner_feeds[0]
        );
        assert_eq!(
            schedule.phases()[1][0].ramp.start_feed,
            schedule.lookahead().corner_feeds[0]
        );
        assert_eq!(schedule.phases()[1][1].ramp.end_feed, Real::zero());
        assert!(schedule.lookahead_plan().all_satisfied());
        assert!(schedule.lookahead_report().all_satisfied());
        assert!(schedule.jerk_report().all_satisfied());
        assert!(
            schedule
                .limits()
                .affine_axis_projection()
                .is_some_and(|projection| projection.all_satisfied())
        );
        assert!(matches!(
            lower_certified_schedule_to_direct_finite_difference(
                &schedule,
                &profile,
                &budget,
                DirectFiniteDifferencePolicy2::interactive(Rational::fraction(1, 1_000).unwrap())
                    .unwrap(),
            ),
            Err(
                crate::direct_motion::DirectMotionError::UnsupportedNonstopSchedule {
                    element_index: 0
                }
            )
        ));

        let lowered = lower_certified_schedule_to_v1(
            &schedule,
            &profile,
            &budget,
            Rational::fraction(1, 1_000).unwrap(),
            ScheduledLoweringLimits::INTERACTIVE,
        )
        .unwrap();
        assert_eq!(
            lowered.points().first().unwrap().steps(),
            [CanonicalStep::new(0); 2]
        );
        assert_eq!(
            lowered.points().last().unwrap().steps(),
            [CanonicalStep::new(3_200), CanonicalStep::new(0)]
        );
        assert!(lowered.executor_preflight().segment_count > 0);
    }

    #[test]
    fn exact_diagonal_g1_uses_dense_axis_projection_and_lowers_to_terminal_steps() {
        let profile = profile_from(&machine_records()).unwrap();
        for axis in profile.axes() {
            assert_eq!(
                axis.effective_velocity_limit_metres_per_second(),
                axis.step_rate_velocity_limit_metres_per_second()
            );
        }
        let source = CurvePath2::try_new(vec![
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(0, 0),
                    CurvePoint2::from_values(3, 4),
                )
                .unwrap(),
            )),
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(3, 4),
                    CurvePoint2::from_values(6, 8),
                )
                .unwrap(),
            )),
        ])
        .unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();

        let projection = schedule.limits().affine_axis_projection().unwrap();
        let scale = Rational::fraction(5, 4).unwrap();
        assert_eq!(
            projection.maximum_path_feed,
            Real::from(
                profile.axes()[1].effective_velocity_limit_metres_per_second()
                    * Rational::from(1_000)
                    * &scale
            )
        );
        assert_eq!(
            projection.maximum_path_acceleration,
            Real::from(
                profile.axes()[1].effective_acceleration_limit_metres_per_second_squared()
                    * Rational::from(1_000)
                    * &scale
            )
        );
        assert_eq!(
            projection.maximum_path_jerk,
            Real::from(
                profile.axes()[1].effective_jerk_limit_metres_per_second_cubed()
                    * Rational::from(1_000)
                    * scale
            )
        );
        assert_eq!(projection.certification.rows.len(), 4);
        assert_eq!(
            projection.certification.rows[0].absolute_axis_derivative,
            Real::from(Rational::fraction(3, 5).unwrap())
        );
        assert_eq!(
            projection.certification.rows[1].absolute_axis_derivative,
            Real::from(Rational::fraction(4, 5).unwrap())
        );
        assert_eq!(projection.feed_bottleneck.span_index, 0);
        assert_eq!(projection.feed_bottleneck.axis_index, 1);
        assert_eq!(projection.acceleration_bottleneck.axis_index, 1);
        assert_eq!(projection.jerk_bottleneck.axis_index, 1);
        assert!(projection.all_satisfied());
        assert_ne!(schedule.lookahead().corner_feeds[0], Real::zero());
        assert!(schedule.lookahead_plan().all_satisfied());
        assert!(schedule.jerk_report().all_satisfied());

        let no_dilation = ScheduledLoweringLimits::try_new_with_timer_dilation(
            ScheduledLoweringLimits::INTERACTIVE.maximum_points(),
            TimerDilationPolicy::try_new(1, 1).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            lower_certified_schedule_to_v1(
                &schedule,
                &profile,
                &budget,
                Rational::fraction(1, 1_000).unwrap(),
                no_dilation,
            ),
            Err(MotionScheduleError::TimerDilationBudgetExceeded {
                maximum_factor_numerator: 1,
                factor_denominator: 1,
                rejection: MotionError::PulseBoundary { axis: 1 },
            })
        ));

        let lowered = lower_certified_schedule_to_v1(
            &schedule,
            &profile,
            &budget,
            Rational::fraction(1, 1_000).unwrap(),
            ScheduledLoweringLimits::INTERACTIVE,
        )
        .unwrap();
        assert_eq!(
            lowered.points().last().unwrap().steps(),
            [CanonicalStep::new(9_600), CanonicalStep::new(12_800)]
        );
        let timer_lattice = lowered.evidence().timer_lattice_schedule();
        assert_eq!(timer_lattice.selected_factor_numerator(), 4_158);
        assert_eq!(timer_lattice.selected_factor_denominator(), 4_096);
        assert_eq!(timer_lattice.maximum_factor_numerator(), 65_536);
        assert_eq!(
            timer_lattice.selected_factor(),
            Rational::fraction(2_079, 2_048).unwrap()
        );
        assert_eq!(timer_lattice.candidate_replays(), 20);
        assert_eq!(
            timer_lattice.unit_factor_rejection(),
            Some(MotionError::PulseBoundary { axis: 1 })
        );
        assert_eq!(
            timer_lattice.predecessor_rejection(),
            Some(MotionError::Rate { axis: 1 })
        );
        assert!(
            timer_lattice.scheduled_total_time_seconds() > timer_lattice.ideal_total_time_seconds()
        );
        assert!(
            timer_lattice.maximum_output_grid_padding_seconds()
                < &(Real::one() / Real::from(profile.timer_ticks_per_second())).unwrap()
        );
        assert!(lowered.executor_preflight().segment_count > 0);
    }

    #[test]
    fn shared_timer_search_is_canonical_complete_and_jointly_minimal() {
        let profile = profile_from(&machine_records()).unwrap();
        let source = CurvePath2::try_new(vec![
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(0, 0),
                    CurvePoint2::from_values(3, 4),
                )
                .unwrap(),
            )),
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(3, 4),
                    CurvePoint2::from_values(6, 8),
                )
                .unwrap(),
            )),
        ])
        .unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();
        let lowered = lower_certified_schedule_to_v1(
            &schedule,
            &profile,
            &budget,
            Rational::fraction(1, 1_000).unwrap(),
            ScheduledLoweringLimits::INTERACTIVE,
        )
        .unwrap();
        let device_1 = DeviceId([1; 16]);
        let device_2 = DeviceId([2; 16]);

        let shared = select_shared_timer_lattice_schedule(
            vec![
                SharedTimerParticipant2::new(device_2, &lowered, &profile),
                SharedTimerParticipant2::new(device_1, &lowered, &profile),
            ],
            TimerDilationPolicy::INTERACTIVE,
        )
        .unwrap();
        let rebuilt = select_shared_timer_lattice_schedule(
            vec![
                SharedTimerParticipant2::new(device_1, &lowered, &profile),
                SharedTimerParticipant2::new(device_2, &lowered, &profile),
            ],
            TimerDilationPolicy::INTERACTIVE,
        )
        .unwrap();

        assert_eq!(shared, rebuilt);
        assert_eq!(shared.selected_factor_numerator(), 4_158);
        assert_eq!(shared.selected_factor_denominator(), 4_096);
        assert_eq!(
            shared.selected_factor(),
            Rational::fraction(2_079, 2_048).unwrap()
        );
        assert_eq!(shared.candidate_rounds(), 20);
        assert_eq!(shared.participant_replays(), 40);
        assert_eq!(shared.candidate_reports().len(), 20);
        assert_eq!(shared.candidate_reports()[0].factor_numerator(), 4_096);
        assert_eq!(
            shared
                .candidate_reports()
                .last()
                .unwrap()
                .factor_numerator(),
            4_157
        );
        assert!(
            shared
                .candidate_reports()
                .iter()
                .all(|round| round.outcomes().len() == 2)
        );
        assert_eq!(
            shared.terminal_tick(),
            lowered.points().last().unwrap().tick()
        );
        assert_eq!(shared.participants()[0].device_id(), device_1);
        assert_eq!(shared.participants()[1].device_id(), device_2);
        for participant in shared.participants() {
            assert_eq!(participant.segments(), lowered.segments());
            assert_eq!(
                participant.unit_factor_outcome(),
                SharedTimerCandidateOutcome2::Rejected(MotionError::PulseBoundary { axis: 1 })
            );
            assert_eq!(
                participant.predecessor_outcome(),
                Some(SharedTimerCandidateOutcome2::Rejected(MotionError::Rate {
                    axis: 1
                }))
            );
            assert_eq!(
                participant.executor_preflight().end_tick.0,
                shared.terminal_tick().get()
            );
        }

        assert!(matches!(
            select_shared_timer_lattice_schedule(
                vec![
                    SharedTimerParticipant2::new(device_1, &lowered, &profile),
                    SharedTimerParticipant2::new(device_1, &lowered, &profile),
                ],
                TimerDilationPolicy::INTERACTIVE,
            ),
            Err(MotionScheduleError::DuplicateSharedTimerParticipant { device_id })
                if device_id == device_1
        ));
        assert!(matches!(
            select_shared_timer_lattice_schedule(Vec::new(), TimerDilationPolicy::INTERACTIVE),
            Err(MotionScheduleError::SharedTimerParticipantsEmpty)
        ));
        assert!(matches!(
            select_shared_timer_lattice_schedule(
                vec![SharedTimerParticipant2::new(device_1, &lowered, &profile)],
                TimerDilationPolicy::try_new(1, 1).unwrap(),
            ),
            Err(MotionScheduleError::SharedTimerDilationBudgetExceeded {
                maximum_factor_numerator: 1,
                factor_denominator: 1,
                device_id,
                rejection: MotionError::PulseBoundary { axis: 1 },
            }) if device_id == device_1
        ));
    }

    #[test]
    fn shared_timer_minimum_is_driven_by_the_strict_participant_only() {
        let mut strict_records = machine_records();
        for record in &mut strict_records {
            if let ConfigurationRecord::Scalar(scalar) = record
                && scalar.fact == ScalarFact::AxisVelocityLimitMetresPerSecond
            {
                // This exact configured ceiling is just below the original
                // 48+48-cycle pulse-rate limit, so both profiles retain the
                // same planner dynamics while only the strict pulse boundary
                // experiences timer-lattice pressure.
                scalar.value = wire_rational(13, 2_000);
                scalar.uncertainty = wire_rational(0, 1);
            }
        }
        let mut relaxed_records = strict_records.clone();
        for record in &mut relaxed_records {
            if let ConfigurationRecord::Binding(binding) = record
                && binding.role == BindingRole::AxisStep
            {
                binding.minimum_active_cycles = 1;
                binding.minimum_inactive_cycles = 1;
            }
        }

        let (strict_profile, strict_schedule, strict_program) = lower_diagonal_g1(&strict_records);
        let (relaxed_profile, relaxed_schedule, relaxed_program) =
            lower_diagonal_g1(&relaxed_records);
        assert_eq!(
            strict_program.points().len(),
            relaxed_program.points().len()
        );
        assert!(
            strict_program
                .points()
                .iter()
                .zip(relaxed_program.points())
                .all(
                    |(strict, relaxed)| strict.ideal_time_seconds() == relaxed.ideal_time_seconds()
                )
        );
        assert!(
            strict_program
                .evidence()
                .timer_lattice_schedule()
                .selected_factor_numerator()
                > TimerDilationPolicy::INTERACTIVE.factor_denominator()
        );
        assert_eq!(
            relaxed_program
                .evidence()
                .timer_lattice_schedule()
                .selected_factor_numerator(),
            TimerDilationPolicy::INTERACTIVE.factor_denominator()
        );

        let relaxed_device = DeviceId([0x11; 16]);
        let strict_device = DeviceId([0x22; 16]);
        let shared = select_shared_timer_lattice_schedule(
            vec![
                SharedTimerParticipant2::new(strict_device, &strict_program, &strict_profile),
                SharedTimerParticipant2::new(relaxed_device, &relaxed_program, &relaxed_profile),
            ],
            TimerDilationPolicy::INTERACTIVE,
        )
        .unwrap();
        assert_eq!(
            shared.selected_factor_numerator(),
            strict_program
                .evidence()
                .timer_lattice_schedule()
                .selected_factor_numerator()
        );
        let strict = shared.participant(strict_device).unwrap();
        let relaxed = shared.participant(relaxed_device).unwrap();
        assert!(matches!(
            strict.unit_factor_outcome(),
            SharedTimerCandidateOutcome2::Rejected(_)
        ));
        assert!(matches!(
            strict.predecessor_outcome(),
            Some(SharedTimerCandidateOutcome2::Rejected(_))
        ));
        assert!(matches!(
            relaxed.unit_factor_outcome(),
            SharedTimerCandidateOutcome2::Accepted(_)
        ));
        assert!(matches!(
            relaxed.predecessor_outcome(),
            Some(SharedTimerCandidateOutcome2::Accepted(_))
        ));
        assert_eq!(strict.ticks(), relaxed.ticks());
        assert_eq!(
            strict.executor_preflight().end_tick,
            relaxed.executor_preflight().end_tick
        );

        let partition_limits = BlockValidationLimits {
            maximum_block_ticks: 10_000_000,
            segment: ValidationLimits {
                maximum_segment_ticks: 10_000_000,
                maximum_steps_per_segment: 100_000,
            },
        };
        let cache_limits = CacheLimits {
            maximum_object_bytes: 64 * 1024 * 1024,
            maximum_chunk_bytes: 4_096,
            maximum_chunks: 100_000,
        };
        let relaxed_policy = MachinePartitionPolicy2::try_new(
            [0x31; 16],
            relaxed_program.capability_digest(),
            relaxed_program.configuration_digest(),
            partition_limits,
            UploadId(0x1111),
            4_096,
            cache_limits,
        )
        .unwrap();
        let strict_policy = MachinePartitionPolicy2::try_new(
            [0x32; 16],
            strict_program.capability_digest(),
            strict_program.configuration_digest(),
            partition_limits,
            UploadId(0x2222),
            4_096,
            cache_limits,
        )
        .unwrap();
        let relaxed_partition =
            package_shared_retimed_scheduled_program(&relaxed_program, relaxed, relaxed_policy)
                .unwrap();
        let strict_partition =
            package_shared_retimed_scheduled_program(&strict_program, strict, strict_policy)
                .unwrap();
        assert_eq!(
            relaxed_partition.terminal_progress().end_tick,
            strict_partition.terminal_progress().end_tick
        );
        assert_eq!(
            relaxed_partition.terminal_progress().end_tick.0,
            shared.terminal_tick().get()
        );

        let evidence = build_shared_timing_evidence(
            &shared,
            vec![
                SharedTimingEvidenceParticipant2::new(
                    strict_device,
                    &strict_schedule,
                    &strict_program,
                    &strict_partition,
                ),
                SharedTimingEvidenceParticipant2::new(
                    relaxed_device,
                    &relaxed_schedule,
                    &relaxed_program,
                    &relaxed_partition,
                ),
            ],
        )
        .unwrap();
        let rebuilt = build_shared_timing_evidence(
            &shared,
            vec![
                SharedTimingEvidenceParticipant2::new(
                    relaxed_device,
                    &relaxed_schedule,
                    &relaxed_program,
                    &relaxed_partition,
                ),
                SharedTimingEvidenceParticipant2::new(
                    strict_device,
                    &strict_schedule,
                    &strict_program,
                    &strict_partition,
                ),
            ],
        )
        .unwrap();
        assert_eq!(evidence, rebuilt);
        assert_eq!(&evidence.encoded()[..8], b"ALMSYN01");
        assert_eq!(evidence.encoded().len(), 104);
        assert_eq!(evidence.participant_count(), 2);
        assert_eq!(evidence.candidate_rounds(), shared.candidate_rounds());
        assert_eq!(evidence.participant_replays(), shared.participant_replays());
        assert!(evidence.transcript_byte_len() > evidence.encoded().len() as u64);
        replay_shared_timing_evidence(
            &evidence,
            &shared,
            vec![
                SharedTimingEvidenceParticipant2::new(
                    relaxed_device,
                    &relaxed_schedule,
                    &relaxed_program,
                    &relaxed_partition,
                ),
                SharedTimingEvidenceParticipant2::new(
                    strict_device,
                    &strict_schedule,
                    &strict_program,
                    &strict_partition,
                ),
            ],
        )
        .unwrap();
        verify_shared_timing_evidence_bytes(evidence.encoded(), evidence.digest()).unwrap();
        let mut corrupted = evidence.encoded().to_vec();
        corrupted[40] ^= 1;
        assert!(matches!(
            verify_shared_timing_evidence_bytes(&corrupted, evidence.digest()),
            Err(SharedTimingEvidenceError::DigestMismatch)
        ));

        let global_template = MachineJobGlobalFacts {
            network_policy: JobNetworkPolicy::NetworkAttended,
            global_timebase_hz: 0,
            duration_ticks: 0,
            source_digest: Digest([0x41; 32]),
            compiler_digest: Digest([0x42; 32]),
            interface_digest: Digest([0x43; 32]),
            policy_digest: Digest([0x44; 32]),
            machine_digest: Digest([0x45; 32]),
            coordinate_epoch_digest: Digest([0x46; 32]),
            safety_policy_digest: Digest([0x47; 32]),
            synchronization_digest: Digest::ZERO,
        };
        let global_policy = SharedGlobalJobCompilePolicy2::try_new(
            global_template,
            UploadId(0x3333),
            1_024,
            CacheLimits {
                maximum_object_bytes: 4 * 1024 * 1024,
                maximum_chunk_bytes: 1_024,
                maximum_chunks: 10_000,
            },
        )
        .unwrap();
        let strict_job_input = SharedScheduledJobParticipant2::new(
            strict_device,
            Digest([0x61; 32]),
            Digest([0x71; 32]),
            Digest([0x91; 32]),
            &strict_schedule,
            &strict_program,
            &strict_profile,
            strict_policy,
        );
        let relaxed_job_input = SharedScheduledJobParticipant2::new(
            relaxed_device,
            Digest([0x62; 32]),
            Digest([0x72; 32]),
            Digest([0x92; 32]),
            &relaxed_schedule,
            &relaxed_program,
            &relaxed_profile,
            relaxed_policy,
        );
        let compiled = compile_shared_scheduled_global_job(
            global_policy,
            TimerDilationPolicy::INTERACTIVE,
            vec![strict_job_input, relaxed_job_input],
        )
        .unwrap();
        let compiled_reversed = compile_shared_scheduled_global_job(
            global_policy,
            TimerDilationPolicy::INTERACTIVE,
            vec![relaxed_job_input, strict_job_input],
        )
        .unwrap();
        assert_eq!(compiled.timing_evidence(), &evidence);
        assert_eq!(
            compiled.global_job().manifest_bytes(),
            compiled_reversed.global_job().manifest_bytes()
        );
        assert_eq!(
            compiled.global_job().global_job_digest(),
            compiled_reversed.global_job().global_job_digest()
        );
        let global = compiled.global_job().policy().global();
        assert_eq!(
            global.global_timebase_hz,
            compiled.retiming().timer_ticks_per_second()
        );
        assert_eq!(
            global.duration_ticks,
            compiled.retiming().terminal_tick().get()
        );
        assert_eq!(
            global.synchronization_digest,
            compiled.timing_evidence().digest()
        );
        assert!(
            compiled
                .global_job()
                .participant_records()
                .iter()
                .all(|participant| participant.error_evidence_digest
                    == compiled.timing_evidence().digest())
        );
    }

    #[test]
    fn shared_timer_v1_rejects_identity_clock_output_and_event_grid_mismatches() {
        let base_records = machine_records();
        let (base_profile, _, base_program) = lower_diagonal_g1(&base_records);
        let base_device = DeviceId([1; 16]);
        let other_device = DeviceId([2; 16]);

        let mut quantum_records = base_records.clone();
        for record in &mut quantum_records {
            if let ConfigurationRecord::Scalar(scalar) = record
                && scalar.fact == ScalarFact::StepperOutputQuantumCycles
            {
                scalar.value = wire_rational(4, 1);
            }
        }
        let (quantum_profile, _, quantum_program) = lower_diagonal_g1(&quantum_records);
        assert!(matches!(
            select_shared_timer_lattice_schedule(
                vec![
                    SharedTimerParticipant2::new(
                        base_device,
                        &base_program,
                        &base_profile,
                    ),
                    SharedTimerParticipant2::new(
                        other_device,
                        &quantum_program,
                        &quantum_profile,
                    ),
                ],
                TimerDilationPolicy::INTERACTIVE,
            ),
            Err(MotionScheduleError::SharedTimerOutputQuantumMismatch {
                device_id,
                expected: 1,
                actual: 4,
            }) if device_id == other_device
        ));
        assert!(matches!(
            select_shared_timer_lattice_schedule(
                vec![SharedTimerParticipant2::new(
                    base_device,
                    &base_program,
                    &quantum_profile,
                )],
                TimerDilationPolicy::INTERACTIVE,
            ),
            Err(MotionScheduleError::SharedTimerParticipantIdentityMismatch { device_id })
                if device_id == base_device
        ));

        let mut clock_records = base_records.clone();
        for record in &mut clock_records {
            if let ConfigurationRecord::Scalar(scalar) = record
                && scalar.fact == ScalarFact::TimerTickHertz
            {
                scalar.value = wire_rational(2_000_000, 1);
            }
        }
        let (clock_profile, _, clock_program) = lower_diagonal_g1(&clock_records);
        assert!(matches!(
            select_shared_timer_lattice_schedule(
                vec![
                    SharedTimerParticipant2::new(
                        base_device,
                        &base_program,
                        &base_profile,
                    ),
                    SharedTimerParticipant2::new(
                        other_device,
                        &clock_program,
                        &clock_profile,
                    ),
                ],
                TimerDilationPolicy::INTERACTIVE,
            ),
            Err(MotionScheduleError::SharedTimerFrequencyMismatch {
                device_id,
                expected: 1_000_000,
                actual: 2_000_000,
            }) if device_id == other_device
        ));

        let alternate_source = representative_metric_path().unwrap();
        let alternate_budget = MachineResolutionBudget2::certify(
            &base_profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let alternate_schedule = certify_jerk_schedule(
            &alternate_source,
            &base_profile,
            &alternate_budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();
        let alternate_program = lower_certified_schedule_to_v1(
            &alternate_schedule,
            &base_profile,
            &alternate_budget,
            Rational::fraction(1, 1_000).unwrap(),
            ScheduledLoweringLimits::INTERACTIVE,
        )
        .unwrap();
        assert_ne!(
            base_program.points().len(),
            alternate_program.points().len()
        );
        assert!(matches!(
            select_shared_timer_lattice_schedule(
                vec![
                    SharedTimerParticipant2::new(
                        base_device,
                        &base_program,
                        &base_profile,
                    ),
                    SharedTimerParticipant2::new(
                        other_device,
                        &alternate_program,
                        &base_profile,
                    ),
                ],
                TimerDilationPolicy::INTERACTIVE,
            ),
            Err(MotionScheduleError::SharedTimerPointCountMismatch {
                device_id,
                expected,
                actual,
            }) if device_id == other_device
                && expected == base_program.points().len()
                && actual == alternate_program.points().len()
        ));
    }

    #[test]
    fn timer_lowering_rounds_each_interval_up_to_the_exact_output_grid() {
        let mut records = machine_records();
        for record in &mut records {
            let ConfigurationRecord::Scalar(scalar) = record else {
                continue;
            };
            if scalar.fact == ScalarFact::StepperOutputQuantumCycles {
                scalar.value = wire_rational(4, 1);
            }
        }
        let profile = profile_from(&records).unwrap();
        let source = representative_metric_path().unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();
        let lowered = lower_certified_schedule_to_v1(
            &schedule,
            &profile,
            &budget,
            Rational::fraction(1, 1_000).unwrap(),
            ScheduledLoweringLimits::INTERACTIVE,
        )
        .unwrap();

        for pair in lowered.points().windows(2) {
            let duration_cycles = pair[1].tick().get() - pair[0].tick().get();
            assert!(duration_cycles.is_multiple_of(4));
            let actual_duration = (Real::from(duration_cycles)
                / Real::from(profile.timer_ticks_per_second()))
            .unwrap();
            let ideal_duration = pair[1].ideal_time_seconds() - pair[0].ideal_time_seconds();
            assert!(actual_duration >= ideal_duration);
        }
        let timer_lattice = lowered.evidence().timer_lattice_schedule();
        assert!(
            timer_lattice.maximum_output_grid_padding_seconds()
                < &(Real::from(4) / Real::from(profile.timer_ticks_per_second())).unwrap()
        );
        assert_eq!(
            lowered.executor_preflight().end_tick.0,
            lowered.points().last().unwrap().tick().get()
        );
    }

    #[test]
    fn exact_reversal_remains_a_full_stop_under_positive_join_policy() {
        let profile = profile_from(&machine_records()).unwrap();
        let source = CurvePath2::try_new(vec![
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(0, 0),
                    CurvePoint2::from_values(1, 0),
                )
                .unwrap(),
            )),
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(1, 0),
                    CurvePoint2::from_values(0, 0),
                )
                .unwrap(),
            )),
        ])
        .unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();

        assert_eq!(schedule.lookahead().corner_feeds, vec![Real::zero()]);
        assert!(
            schedule
                .lookahead_plan()
                .positive_node_components
                .is_empty()
        );
        assert!(
            schedule
                .lookahead_plan()
                .span_transitions
                .iter()
                .all(Option::is_none)
        );
        assert!(schedule.phases().iter().all(|phases| phases.len() == 4));
        assert!(schedule.lookahead_plan().all_satisfied());
        assert!(schedule.jerk_report().all_satisfied());
    }

    #[test]
    fn curvature_bearing_g1_join_remains_a_full_stop() {
        let profile = profile_from(&machine_records()).unwrap();
        let source = CurvePath2::try_new(vec![
            Curve2::new(CurveGeometry2::Line(
                LineSeg2::try_new(
                    CurvePoint2::from_values(0, 1),
                    CurvePoint2::from_values(1, 1),
                )
                .unwrap(),
            )),
            Curve2::new(CurveGeometry2::CircularArc(
                CircularArc2::try_from_center(
                    CurvePoint2::from_values(1, 1),
                    CurvePoint2::from_values(2, 0),
                    CurvePoint2::from_values(1, 0),
                    true,
                )
                .unwrap(),
            )),
        ])
        .unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();

        assert_eq!(schedule.lookahead().corner_feeds, vec![Real::zero()]);
        assert!(
            schedule
                .lookahead_plan()
                .positive_node_components
                .is_empty()
        );
        assert!(schedule.phases().iter().all(|phases| phases.len() == 4));
        assert!(schedule.lookahead_plan().all_satisfied());
        assert!(schedule.jerk_report().all_satisfied());
    }

    #[test]
    fn retained_cubic_gets_certified_chords_with_an_exact_stop_at_every_join() {
        let profile = profile_from(&machine_records()).unwrap();
        let source = representative_curve_path().unwrap();
        let source_allocation = Rational::fraction(1, 100).unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            source_allocation.clone(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();

        assert_eq!(schedule.source(), &source);
        assert_eq!(schedule.metric_path().spans().len(), 3);
        assert_eq!(
            schedule.metric_path().maximum_source_error_mm_exact(),
            &source_allocation
        );
        assert!(schedule.metric_path().spans()[2].is_approximated());
        assert!(schedule.metric_path().spans()[2].motion_element_count() > 1);
        assert_eq!(
            schedule.route().len(),
            schedule.metric_path().path().curves().len()
        );
        assert!(schedule.route().len() > source.curves().len());
        assert!(
            schedule
                .lookahead()
                .corner_feeds
                .iter()
                .all(|feed| feed == &Real::zero())
        );
        assert!(schedule.lookahead_plan().all_satisfied());
        assert!(
            schedule
                .acceleration_lookahead_plan()
                .effective_node_feed_limits
                .iter()
                .all(|feed| feed == &Real::zero())
        );
        assert!(
            schedule
                .acceleration_lookahead_plan()
                .forward_node_feeds
                .iter()
                .all(|feed| feed == &Real::zero())
        );
        assert!(schedule.phases().iter().all(|phases| {
            phases.len() == 4
                && phases[0].ramp.start_feed == Real::zero()
                && phases[3].ramp.end_feed == Real::zero()
        }));
        assert!(schedule.lookahead_report().all_satisfied());
        assert!(schedule.jerk_report().all_satisfied());

        let lowered = lower_certified_schedule_to_v1(
            &schedule,
            &profile,
            &budget,
            Rational::fraction(1, 1_000).unwrap(),
            ScheduledLoweringLimits::INTERACTIVE,
        )
        .unwrap();
        assert_eq!(lowered.source(), &source);
        assert_eq!(lowered.metric_path(), schedule.metric_path());
        assert_eq!(
            lowered.evidence().maximum_source_to_motion_error_mm_exact(),
            &source_allocation
        );
        assert_eq!(
            lowered.points().last().unwrap().steps(),
            [CanonicalStep::new(19_200), CanonicalStep::new(0)]
        );
        assert!(
            lowered
                .points()
                .iter()
                .any(|point| point.source_element() == 2)
        );
        assert!(
            lowered
                .points()
                .iter()
                .map(|point| point.motion_element())
                .max()
                .is_some_and(|maximum| maximum + 1 == schedule.route().len())
        );
        let zero_source_budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            lower_certified_schedule_to_v1(
                &schedule,
                &profile,
                &zero_source_budget,
                Rational::fraction(1, 1_000).unwrap(),
                ScheduledLoweringLimits::INTERACTIVE,
            ),
            Err(MotionScheduleError::SourceApproximationAllocationExceeded)
        ));

        let partition_policy = MachinePartitionPolicy2::try_new(
            [0x42; 16],
            profile.capability_digest(),
            profile.configuration_digest(),
            BlockValidationLimits {
                maximum_block_ticks: 10_000_000,
                segment: ValidationLimits {
                    maximum_segment_ticks: 10_000_000,
                    maximum_steps_per_segment: 100_000,
                },
            },
            UploadId(0x2233_4455_6677_8899),
            700,
            CacheLimits {
                maximum_object_bytes: 4 * 1024 * 1024,
                maximum_chunk_bytes: 1_024,
                maximum_chunks: 10_000,
            },
        )
        .unwrap();
        let partition = package_canonical_scheduled_program(&lowered, partition_policy).unwrap();
        let evidence = build_canonical_schedule_evidence(&schedule, &lowered, &partition).unwrap();
        let rebuilt = build_canonical_schedule_evidence(&schedule, &lowered, &partition).unwrap();

        assert_eq!(evidence, rebuilt);
        assert_eq!(&evidence.encoded()[..8], b"ALMEVD03");
        assert_ne!(evidence.source_digest(), evidence.metric_path_digest());
        assert!(!evidence.source_approximation_digest().is_zero());
        replay_canonical_schedule_evidence(&evidence, &schedule, &lowered, &partition).unwrap();
    }

    #[test]
    fn interpolation_point_allocation_is_caller_bounded() {
        let profile = profile_from(&machine_records()).unwrap();
        let source = representative_metric_path().unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();
        let limits = ScheduledLoweringLimits::try_new(2).unwrap();

        assert_eq!(limits.maximum_points(), 2);
        assert!(matches!(
            lower_certified_schedule_to_v1(
                &schedule,
                &profile,
                &budget,
                Rational::fraction(1, 1_000).unwrap(),
                limits,
            ),
            Err(MotionScheduleError::PointBudgetExceeded {
                maximum: 2,
                required,
            }) if required > 2
        ));
        assert!(matches!(
            ScheduledLoweringLimits::try_new(1),
            Err(MotionScheduleError::InvalidLoweringLimits)
        ));
        assert!(matches!(
            TimerDilationPolicy::try_new(0, 1),
            Err(MotionScheduleError::InvalidTimerDilationPolicy)
        ));
        assert!(matches!(
            TimerDilationPolicy::try_new(2, 1),
            Err(MotionScheduleError::InvalidTimerDilationPolicy)
        ));
    }

    #[test]
    fn exact_source_envelope_must_fit_conservative_machine_travel() {
        let mut records = machine_records();
        for record in &mut records {
            if let ConfigurationRecord::Scalar(scalar) = record
                && scalar.instance == 0
                && scalar.fact == ScalarFact::AxisPositionMaximumMetres
            {
                scalar.value = wire_rational(7, 1_000);
            }
        }
        let profile = profile_from(&records).unwrap();
        let source = representative_metric_path().unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            certify_jerk_schedule(
                &source,
                &profile,
                &budget,
                MetricPathApproximationLimits2::INTERACTIVE,
            ),
            Err(MotionScheduleError::TravelEnvelopeExceeded {
                axis: 0,
                boundary: TravelBoundary::Maximum,
            })
        ));
    }

    #[test]
    fn rounded_canonical_points_must_remain_inside_usable_travel() {
        let mut records = machine_records();
        for record in &mut records {
            if let ConfigurationRecord::Scalar(scalar) = record
                && scalar.instance == 0
            {
                match scalar.fact {
                    ScalarFact::AxisTravelMetresPerOutputTurn => {
                        scalar.value = wire_rational(3, 500);
                    }
                    ScalarFact::AxisPositionMaximumMetres => {
                        // 8.0003 mm admits the exact 8 mm endpoint, but the
                        // selected 1600/3 steps/mm lattice rounds it outward to
                        // 8.000625 mm and must therefore fail separately.
                        scalar.value = wire_rational(80_003, 10_000_000);
                    }
                    _ => {}
                }
            }
        }
        let profile = profile_from(&records).unwrap();
        let source = representative_metric_path().unwrap();
        let budget = MachineResolutionBudget2::certify(
            &profile,
            Rational::fraction(1, 10).unwrap(),
            Rational::zero(),
            Rational::fraction(1, 100).unwrap(),
        )
        .unwrap();
        let schedule = certify_jerk_schedule(
            &source,
            &profile,
            &budget,
            MetricPathApproximationLimits2::INTERACTIVE,
        )
        .unwrap();
        assert!(matches!(
            lower_certified_schedule_to_v1(
                &schedule,
                &profile,
                &budget,
                Rational::fraction(1, 1_000).unwrap(),
                ScheduledLoweringLimits::INTERACTIVE,
            ),
            Err(MotionScheduleError::CanonicalTravelExceeded {
                axis: 0,
                boundary: TravelBoundary::Maximum,
                ..
            })
        ));
    }

    #[test]
    fn missing_facts_and_nonpositive_uncertainty_bounds_fail_closed() {
        let mut missing = machine_records();
        missing.retain(|record| {
            !matches!(
                record,
                ConfigurationRecord::Scalar(scalar)
                    if scalar.instance == 1
                        && scalar.fact == ScalarFact::AxisFollowingErrorMetres
            )
        });
        assert!(matches!(
            profile_from(&missing),
            Err(MachineProfileError::MissingScalar {
                instance: 1,
                fact: ScalarFact::AxisFollowingErrorMetres,
            })
        ));

        let mut nonpositive = machine_records();
        for record in &mut nonpositive {
            if let ConfigurationRecord::Scalar(scalar) = record
                && scalar.instance == 0
                && scalar.fact == ScalarFact::AxisCalibrationScale
            {
                scalar.uncertainty = wire_rational(1, 1);
            }
        }
        assert!(matches!(
            profile_from(&nonpositive),
            Err(MachineProfileError::NonPositiveLowerBound {
                instance: 0,
                fact: ScalarFact::AxisCalibrationScale,
            })
        ));
    }
}
