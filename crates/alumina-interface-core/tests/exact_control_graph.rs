use alumina_interface_core::graph::{
    ExternalStreamSample, GraphSimulation, GraphSimulationLimits, GraphTraceEntryKind, GraphValue,
    RepresentativeControlSignal, WireEndpoint, compile_representative_exact_control_graph,
    replay_graph_trace, simulate_graph,
};
use hyperreal::Rational;

fn rational_trace(simulation: &GraphSimulation, endpoint: WireEndpoint) -> Vec<Rational> {
    simulation
        .entries()
        .iter()
        .filter(|entry| entry.endpoint() == endpoint)
        .map(|entry| match entry.value().value() {
            GraphValue::ExactRational(value) => value.clone(),
            value => panic!("unexpected trace value {value:?}"),
        })
        .collect()
}

fn boolean_trace(simulation: &GraphSimulation, endpoint: WireEndpoint) -> Vec<bool> {
    simulation
        .entries()
        .iter()
        .filter(|entry| entry.endpoint() == endpoint)
        .map(|entry| match entry.value().value() {
            GraphValue::Boolean(value) => *value,
            value => panic!("unexpected trace value {value:?}"),
        })
        .collect()
}

#[test]
fn shared_multirate_exact_pid_is_visible_deterministic_and_replayable() {
    let fixture = compile_representative_exact_control_graph().unwrap();
    let simulation = fixture.simulation();
    let limits = GraphSimulationLimits::interactive();

    let mut reversed_inputs: Vec<_> = simulation
        .entries()
        .iter()
        .filter(|entry| entry.kind() == GraphTraceEntryKind::ExternalSource)
        .map(|entry| {
            ExternalStreamSample::new(
                entry.endpoint(),
                entry.clock_tick(),
                entry.sequence(),
                entry.value().clone(),
            )
        })
        .collect();
    reversed_inputs.reverse();
    let reversed = simulate_graph(
        fixture.document(),
        fixture.registry(),
        simulation.horizon(),
        &reversed_inputs,
        limits,
    )
    .unwrap();
    assert_eq!(&reversed, simulation);

    assert_eq!(
        rational_trace(
            simulation,
            RepresentativeControlSignal::IntegralPrior.endpoint(),
        ),
        [0, 3, 5, 6, 6, 6].map(Rational::from)
    );
    assert_eq!(
        rational_trace(
            simulation,
            RepresentativeControlSignal::ClampedController.endpoint(),
        ),
        [5, 5, 4, 2, 3, 3].map(Rational::from)
    );
    assert_eq!(
        rational_trace(
            simulation,
            RepresentativeControlSignal::PermittedOutput.endpoint(),
        ),
        [5, 5, 4, 0, 0, 0].map(Rational::from)
    );
    assert_eq!(
        boolean_trace(
            simulation,
            RepresentativeControlSignal::ExternalPermit.endpoint(),
        ),
        [true, true, true, true, false, false]
    );
    assert_eq!(
        boolean_trace(
            simulation,
            RepresentativeControlSignal::MeasurementWithinRange.endpoint(),
        ),
        [true, true, true, false, false, false]
    );
    assert_eq!(
        boolean_trace(
            simulation,
            RepresentativeControlSignal::CombinedPermit.endpoint(),
        ),
        [true, true, true, false, false, false]
    );

    assert_eq!(
        simulation.graph_digest().0,
        [
            0x96, 0xa3, 0x34, 0x82, 0x64, 0xa9, 0xb6, 0x5d, 0x26, 0x7b, 0x45, 0xf9, 0xa6, 0x41,
            0x9a, 0x44, 0xee, 0x60, 0x47, 0x3f, 0xd9, 0x61, 0xab, 0xcf, 0x44, 0x36, 0x29, 0x5e,
            0x10, 0xb3, 0x73, 0x5f,
        ]
    );
    assert_eq!(
        simulation.registry_digest().0,
        [
            0x3c, 0xcd, 0x0a, 0xa5, 0xdb, 0xc2, 0xf7, 0x45, 0xb8, 0x97, 0xca, 0x06, 0xe9, 0xa7,
            0x57, 0x17, 0x36, 0x5c, 0xd9, 0x77, 0xab, 0x30, 0x3e, 0x3c, 0x1c, 0xa4, 0x8b, 0xdc,
            0xf8, 0xf7, 0xc5, 0x25,
        ]
    );
    assert_eq!(fixture.trace().bytes().len(), 8_292);
    assert_eq!(
        fixture.trace().digest().0,
        [
            0xa8, 0x9c, 0x8b, 0xfa, 0x87, 0xe7, 0xdc, 0xed, 0x32, 0x8d, 0x6c, 0xb6, 0x58, 0x3a,
            0x10, 0xa6, 0x18, 0x43, 0x17, 0x75, 0xbd, 0x3a, 0x51, 0xd2, 0xab, 0x61, 0xa5, 0x69,
            0x97, 0xd4, 0xc1, 0x77,
        ]
    );
    let replay = replay_graph_trace(
        fixture.trace().bytes(),
        fixture.document(),
        fixture.registry(),
        limits,
    )
    .unwrap();
    assert_eq!(replay.simulation(), simulation);
    assert_eq!(replay.encoding().digest(), fixture.trace().digest());
}
