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

#[test]
fn shared_multirate_exact_pid_is_visible_deterministic_and_replayable() {
    let fixture = compile_representative_exact_control_graph().unwrap();
    let simulation = fixture.simulation();
    let limits = GraphSimulationLimits::interactive();

    let mut reversed_inputs: Vec<_> = simulation
        .entries()
        .iter()
        .filter(|entry| entry.kind() == GraphTraceEntryKind::ExternalInput)
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
        simulation.graph_digest().0,
        [
            0xfb, 0x17, 0x3f, 0xb3, 0x0b, 0xc5, 0xe0, 0x42, 0x69, 0xca, 0xea, 0x43, 0x9d, 0xea,
            0x8f, 0xa4, 0x55, 0x05, 0x01, 0x42, 0xfa, 0xc3, 0xa4, 0xaf, 0xc7, 0x8f, 0x5f, 0xd1,
            0x6e, 0x7a, 0xc5, 0x9a,
        ]
    );
    assert_eq!(
        simulation.registry_digest().0,
        [
            0x6b, 0xb6, 0xf8, 0x14, 0x94, 0x1b, 0x63, 0x2a, 0xc5, 0xc9, 0x85, 0x8f, 0xbb, 0xfe,
            0x59, 0x9f, 0xe8, 0xfe, 0xbb, 0x3a, 0x04, 0xb4, 0xdc, 0xc7, 0xbf, 0x4f, 0xbc, 0x8a,
            0xc2, 0xf6, 0x15, 0x37,
        ]
    );
    assert_eq!(fixture.trace().bytes().len(), 7_836);
    assert_eq!(
        fixture.trace().digest().0,
        [
            0x4d, 0x9b, 0x63, 0x63, 0x3b, 0xe3, 0xaf, 0xc6, 0x58, 0xca, 0xc8, 0xd6, 0x47, 0x5d,
            0x6e, 0xde, 0x60, 0x25, 0x68, 0xab, 0x08, 0x4d, 0xe0, 0x05, 0xac, 0x5d, 0xd2, 0xdf,
            0xcb, 0x75, 0x42, 0xa3,
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
