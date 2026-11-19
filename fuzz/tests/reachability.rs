use alumina_config::ScalarFact;
use alumina_interface_core::MachineProfileError;
use alumina_interface_fuzz::{Outcome, ProfileFailure, machine_lowering, machine_profile, profile};
use hyperreal::Rational;
#[test]
fn structured_profiles_reach_exact_facts_and_invalid_cases_stay_distinct() {
    for micro in [0, 1, 7, 15, 31] {
        assert_eq!(machine_profile(&[0, 0, micro]), Outcome::Profiled);
    }
    assert!(matches!(
        machine_profile(&[64]),
        Outcome::InvalidConfiguration | Outcome::InvalidProfile
    ));
    assert_eq!(
        machine_profile(&[128, 0, 0, 1]),
        Outcome::InvalidConfiguration
    );
    assert_eq!(machine_profile(&[0; 65]), Outcome::SkippedOversize);
}
#[test]
fn lowering_reaches_real_cached_execution_and_rejects_too_small_budgets() {
    assert_eq!(machine_lowering(&[0, 0, 15]), Outcome::Replayed);
    assert_eq!(
        machine_lowering(&[0, 0, 15, 0, 0, 1]),
        Outcome::BudgetRejected
    );
    assert_eq!(
        machine_lowering(&[0, 0, 15, 0, 0, 0, 0, 0, 1]),
        Outcome::LoweringRejected
    );
}

#[test]
fn fresh_process_reaches_lowering_without_warm_parent_state() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "lowering_reaches_real_cached_execution_and_rejects_too_small_budgets",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[test]
fn unreduced_generated_uncertainty_preserves_its_exact_value() {
    // Original profile crash: 0/2 must become the canonical exact zero.
    assert_eq!(machine_profile(&[0, 0, 0, 0, 1]), Outcome::Profiled);
    // Original lowering crash: integer microsteps forbid any uncertainty.
    // ConfigurationRecord::validate_shape requires_integer is authoritative.
    assert_eq!(
        machine_lowering(&[0, 255, 224, 254, 245]),
        Outcome::InvalidConfiguration
    );
    assert_eq!(
        machine_profile(&[0, 0, 15, 6, 1]),
        Outcome::InvalidConfiguration
    );
    // Calibration admits the same reducible rational uncertainty, scaled to ppm.
    assert_eq!(machine_profile(&[32, 0, 15, 6, 1]), Outcome::Profiled);
    assert_eq!(machine_lowering(&[32, 0, 15, 6, 1]), Outcome::Replayed);
}

#[test]
fn bounded_structured_profile_grammar_preserves_exact_uncertainty() {
    for axis in 0..2 {
        for micro in 0..32 {
            for numerator in 0..8 {
                for denominator in 0..4 {
                    let data = [32, axis, micro, numerator, denominator];
                    assert_eq!(machine_profile(&data), Outcome::Profiled, "{data:?}");
                    let data = [0, axis, micro, numerator, denominator];
                    let expected = if numerator == 0 {
                        Outcome::Profiled
                    } else {
                        Outcome::InvalidConfiguration
                    };
                    assert_eq!(machine_profile(&data), expected, "{data:?}");
                }
            }
        }
    }
}

#[test]
fn travel_denominator_must_stay_positive_on_both_axes() {
    let mut data = *include_bytes!("../seeds/machine_profile/travel_reaches_zero");
    for axis in 0..2 {
        data[1] = axis;
        for code in 0..=u8::MAX {
            data[10] = code;
            let expected = if code < 128 || code % 16 < 8 {
                Outcome::Profiled
            } else {
                Outcome::InvalidProfile
            };
            assert_eq!(machine_profile(&data), expected, "{data:?}");
        }
    }
}

#[test]
fn travel_uncertainty_survives_profile_and_cached_execution() {
    let one_eighth = include_bytes!("../seeds/machine_profile/travel_one_eighth");
    let reaches_zero = include_bytes!("../seeds/machine_profile/travel_reaches_zero");
    let one_ppm = include_bytes!("../seeds/machine_lowering/travel_one_ppm");
    assert_eq!(machine_profile(one_eighth), Outcome::Profiled);
    assert_eq!(machine_lowering(reaches_zero), Outcome::InvalidProfile);
    assert_eq!(machine_lowering(one_ppm), Outcome::Replayed);
    let mut tight_budget = *one_ppm;
    tight_budget[5] = 1;
    assert_eq!(machine_lowering(&tight_budget), Outcome::BudgetRejected);
}

#[test]
fn one_eighth_travel_uncertainty_selects_opposite_density_endpoints() {
    // 200 steps/turn * 16 microsteps = 3,200 microsteps per turn.
    // Travel is [7/4000, 9/4000] m/turn; calibration is exactly one.
    for axis in 0..2 {
        let data = [32, axis, 15, 0, 0, 0, 0, 0, 0, 0, 129];
        let result = profile(&data, false).unwrap();
        let selected = &result.axes()[usize::from(axis)];
        let travel = selected.travel_metres_per_output_turn();
        assert_eq!(travel.lower(), &Rational::fraction(7, 4_000).unwrap());
        assert_eq!(travel.upper(), &Rational::fraction(9, 4_000).unwrap());
        let density = selected.command_density_steps_per_metre();
        assert_eq!(density.lower(), &Rational::fraction(12_800_000, 9).unwrap());
        assert_eq!(density.upper(), &Rational::fraction(12_800_000, 7).unwrap());
        let mm = selected.command_density_steps_per_millimetre();
        assert_eq!(mm.lower(), &Rational::fraction(12_800, 9).unwrap());
        assert_eq!(mm.upper(), &Rational::fraction(12_800, 7).unwrap());
    }
}

#[test]
fn one_ppm_travel_and_three_ppm_calibration_compose_exactly() {
    let input = include_bytes!("../seeds/machine_lowering/travel_one_ppm");
    let result = profile(input, false).unwrap();
    let axis = &result.axes()[0];
    let travel = axis.travel_metres_per_output_turn();
    assert_eq!(
        travel.lower(),
        &Rational::fraction(999_999, 500_000_000).unwrap()
    );
    assert_eq!(
        travel.upper(),
        &Rational::fraction(1_000_001, 500_000_000).unwrap()
    );
    let density = axis.command_density_steps_per_millimetre();
    assert_eq!(
        density.lower(),
        &Rational::fraction(1_600 * 999_997, 1_000_001).unwrap()
    );
    assert_eq!(
        density.upper(),
        &Rational::fraction(1_600 * 1_000_003, 999_999).unwrap()
    );
}

#[test]
fn zero_or_negative_travel_lower_bound_identifies_the_fact() {
    for axis in 0..2 {
        for code in [136, 137] {
            let input = [32, axis, 15, 0, 0, 0, 0, 0, 0, 0, code];
            assert!(matches!(profile(&input, false),
                Err(ProfileFailure::Profile(MachineProfileError::NonPositiveLowerBound {
                    instance, fact: ScalarFact::AxisTravelMetresPerOutputTurn,
                })) if instance == u16::from(axis)));
        }
    }
}
