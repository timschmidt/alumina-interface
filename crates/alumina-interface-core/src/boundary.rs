//! Disjoint exact, measured, canonical machine, and lossy display values.

use core::fmt;
use core::marker::PhantomData;

use alumina_machine_ir::{ExecutionSegment, StreamTick};
use hyperreal::{Problem, Rational, Real};

/// A compile-time physical unit carried by exact and measured values.
pub trait Unit: Copy + fmt::Debug + Eq + 'static {
    /// Short human-readable unit symbol.
    const SYMBOL: &'static str;
}

/// Millimetres in machine, work, or design space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Millimetres {}

impl Unit for Millimetres {
    const SYMBOL: &'static str = "mm";
}

/// Seconds in source, schedule, or measurement space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Seconds {}

impl Unit for Seconds {
    const SYMBOL: &'static str = "s";
}

/// An exact design/CAM value whose unit is part of its Rust type.
///
/// Renderer values have no conversion into this type. Decimal text is parsed
/// directly by Hyperreal, without passing through a primitive float.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactValue<U: Unit> {
    value: Real,
    unit: PhantomData<U>,
}

impl<U: Unit> ExactValue<U> {
    /// Retain an existing exact Hyperreal value in this physical unit.
    pub fn from_real(value: Real) -> Self {
        Self {
            value,
            unit: PhantomData,
        }
    }

    /// Retain an exact rational in this physical unit.
    pub fn from_rational(value: Rational) -> Self {
        Self::from_real(Real::from(value))
    }

    /// Parse exact decimal or rational text directly into the CAM domain.
    pub fn parse_decimal(value: &str) -> Result<Self, BoundaryError> {
        value
            .parse::<Real>()
            .map(Self::from_real)
            .map_err(BoundaryError::ExactValue)
    }

    /// Borrow the exact scalar for geometry and CAM algorithms.
    pub const fn as_real(&self) -> &Real {
        &self.value
    }

    /// Return this type's physical-unit symbol.
    pub const fn unit_symbol() -> &'static str {
        U::SYMBOL
    }
}

/// A measured value represented by exact rational lower and upper bounds.
///
/// Measurement uncertainty therefore remains explicit and cannot masquerade as
/// either exact design intent or a renderer approximation.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundedMeasurement<U: Unit> {
    lower: Rational,
    upper: Rational,
    unit: PhantomData<U>,
}

impl<U: Unit> BoundedMeasurement<U> {
    /// Construct the closed interval `nominal ± uncertainty`.
    pub fn from_nominal_uncertainty(
        nominal: Rational,
        uncertainty: Rational,
    ) -> Result<Self, BoundaryError> {
        if uncertainty.is_negative() {
            return Err(BoundaryError::NegativeUncertainty);
        }
        Ok(Self {
            lower: nominal.clone() - uncertainty.clone(),
            upper: nominal + uncertainty,
            unit: PhantomData,
        })
    }

    /// Borrow the exact lower bound.
    pub const fn lower(&self) -> &Rational {
        &self.lower
    }

    /// Borrow the exact upper bound.
    pub const fn upper(&self) -> &Rational {
        &self.upper
    }

    /// Return this type's physical-unit symbol.
    pub const fn unit_symbol() -> &'static str {
        U::SYMBOL
    }
}

/// One canonical signed motor/output lattice count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CanonicalStep(i64);

impl CanonicalStep {
    /// Construct an already-quantized canonical count.
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Return the canonical integer count.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// One canonical stream-relative firmware tick.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct CanonicalCycle(u64);

impl CanonicalCycle {
    /// Construct an already-quantized stream-relative tick.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the canonical integer tick.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Construct one canonical firmware motion segment from machine-domain values.
pub fn canonical_motion_segment<const AXES: usize>(
    start: CanonicalCycle,
    end: CanonicalCycle,
    delta: [CanonicalStep; AXES],
) -> Result<ExecutionSegment<AXES>, BoundaryError> {
    if end <= start {
        return Err(BoundaryError::EmptyOrReversedCycleRange);
    }
    Ok(ExecutionSegment {
        start_tick: StreamTick(start.get()),
        end_tick: StreamTick(end.get()),
        delta_steps: delta.map(CanonicalStep::get),
        flags: 0,
    })
}

/// A finite lossy scalar authorized only for presentation.
///
/// ```compile_fail
/// use alumina_interface_core::{ExactValue, Millimetres, project_for_display};
///
/// fn accepts_cam_value(_: ExactValue<Millimetres>) {}
/// let exact = ExactValue::<Millimetres>::parse_decimal("0.1").unwrap();
/// let display = project_for_display(&exact).unwrap();
/// accepts_cam_value(display);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(transparent)]
pub struct DisplayScalar(f64);

impl DisplayScalar {
    /// Return the finite primitive value at a presentation API boundary.
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// Explicitly and lossily project an exact value for display.
///
/// There is intentionally no reverse conversion.
pub fn project_for_display<U: Unit>(value: &ExactValue<U>) -> Result<DisplayScalar, BoundaryError> {
    let projected = value
        .as_real()
        .to_f64_lossy()
        .filter(|value| value.is_finite())
        .ok_or(BoundaryError::DisplayProjection)?;
    Ok(DisplayScalar(projected))
}

/// Rejection at one named value-domain boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundaryError {
    /// Hyperreal rejected exact input text.
    ExactValue(Problem),
    /// A measurement radius cannot be negative.
    NegativeUncertainty,
    /// An exact value could not be represented as a finite display `f64`.
    DisplayProjection,
    /// A canonical segment must have a strictly positive tick duration.
    EmptyOrReversedCycleRange,
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExactValue(error) => write!(formatter, "invalid exact value: {error}"),
            Self::NegativeUncertainty => formatter.write_str("measurement uncertainty is negative"),
            Self::DisplayProjection => {
                formatter.write_str("exact value has no finite display projection")
            }
            Self::EmptyOrReversedCycleRange => {
                formatter.write_str("canonical cycle range is empty or reversed")
            }
        }
    }
}

impl std::error::Error for BoundaryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_input_stays_exact_until_named_display_projection() {
        let value = ExactValue::<Millimetres>::parse_decimal("0.1").unwrap();
        assert_eq!(
            value.as_real().exact_rational_ref(),
            Some(&Rational::fraction(1, 10).unwrap())
        );
        assert_eq!(project_for_display(&value).unwrap().get(), 0.1_f64);
    }

    #[test]
    fn measurement_keeps_exact_closed_bounds() {
        let measurement = BoundedMeasurement::<Millimetres>::from_nominal_uncertainty(
            Rational::fraction(10, 1).unwrap(),
            Rational::fraction(1, 20).unwrap(),
        )
        .unwrap();
        assert_eq!(measurement.lower(), &Rational::fraction(199, 20).unwrap());
        assert_eq!(measurement.upper(), &Rational::fraction(201, 20).unwrap());
    }

    #[test]
    fn measurement_interval_property_holds_for_integer_fixture_grid() {
        for nominal in -32_i64..=32 {
            for uncertainty in 0_u64..=8 {
                let nominal = Rational::from(nominal);
                let uncertainty = Rational::fraction(
                    i64::try_from(uncertainty).expect("small fixture value fits i64"),
                    8,
                )
                .unwrap();
                let measurement = BoundedMeasurement::<Millimetres>::from_nominal_uncertainty(
                    nominal.clone(),
                    uncertainty.clone(),
                )
                .unwrap();

                assert!(measurement.lower() <= measurement.upper());
                assert_eq!(
                    measurement.upper().clone() - measurement.lower().clone(),
                    uncertainty.clone() * Rational::from(2),
                );
                assert_eq!(
                    (measurement.upper().clone() + measurement.lower().clone()) / Rational::from(2),
                    nominal,
                );
            }
        }
    }

    #[test]
    fn canonical_segment_is_already_integer_machine_ir() {
        let segment = canonical_motion_segment(
            CanonicalCycle::new(10),
            CanonicalCycle::new(30),
            [CanonicalStep::new(7), CanonicalStep::new(-3)],
        )
        .unwrap();
        assert_eq!(segment.start_tick, StreamTick(10));
        assert_eq!(segment.end_tick, StreamTick(30));
        assert_eq!(segment.delta_steps, [7, -3]);
        assert_eq!(segment.flags, 0);
    }

    #[test]
    fn invalid_uncertainty_and_timing_fail_closed() {
        assert_eq!(
            BoundedMeasurement::<Seconds>::from_nominal_uncertainty(
                Rational::zero(),
                Rational::from(-1),
            ),
            Err(BoundaryError::NegativeUncertainty)
        );
        assert_eq!(
            canonical_motion_segment::<1>(
                CanonicalCycle::new(4),
                CanonicalCycle::new(4),
                [CanonicalStep::new(1)],
            ),
            Err(BoundaryError::EmptyOrReversedCycleRange)
        );
    }
}
