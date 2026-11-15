use alumina_interface_fuzz::{Outcome, machine_lowering, machine_profile};
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
