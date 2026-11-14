use alumina_interface_client::{
    Response,
    network::{NetworkClientError, NetworkClientPhase, NetworkProvisioningMachine, NetworkUpdate},
    worker::{
        ConfigurationPhaseSnapshot as Phase, ConfigurationSummarySnapshot as Summary,
        ConfigurationWorkerSnapshot as Snapshot, WorkerContractError, WorkerNetworkScanSnapshot,
    },
};
use alumina_net::provisioning::{
    MAX_NETWORK_SCAN_RESULTS, NetworkAuthentication, NetworkScanEntry, NetworkScanResult,
};
use alumina_protocol::{Operation, StatusCode};
fn scan(transaction: u64) -> NetworkScanResult {
    let entries: Vec<_> = (0..MAX_NETWORK_SCAN_RESULTS)
        .map(|i| {
            NetworkScanEntry::try_new(
                &format!("Fixture-{i}"),
                NetworkAuthentication::Wpa2Personal,
                (i + 1) as u8,
                -(30 + i as i8),
                [2, 0, 0, 0, 0, (i + 1) as u8],
            )
            .unwrap()
        })
        .collect();
    NetworkScanResult::try_new(transaction, 13, 17, 123456, &entries, true).unwrap()
}
#[test]
fn owned_scan_preserves_full_canonical_payload_snapshot_and_independent_clones() {
    let mut machine = NetworkProvisioningMachine::starting_at(42).unwrap();
    let transaction = machine.request_scan().unwrap();
    let request = machine.next_request().unwrap().unwrap();
    assert_eq!(request.operation(), Operation::NetworkScan);
    let expected = scan(transaction);
    let encoded = expected.encode();
    let expected_snapshot =
        serde_json::to_string(&WorkerNetworkScanSnapshot::from_scan(expected)).unwrap();
    let update = machine
        .accept_response(&Response {
            status: StatusCode::Ok,
            body: encoded.to_vec(),
        })
        .unwrap();
    assert_eq!(machine.phase(), NetworkClientPhase::Idle);
    assert_eq!(machine.latest_scan(), Some(expected));
    let mut cloned = update.clone();
    assert_eq!(update, cloned);
    assert_eq!(format!("{update:?}"), format!("Scan({expected:?})"));
    machine.reset(99).unwrap();
    assert!(machine.latest_scan().is_none());
    let (NetworkUpdate::Scan(original), NetworkUpdate::Scan(other)) = (&update, &mut cloned) else {
        panic!("expected scans")
    };
    assert!(!std::ptr::eq(original.as_ref(), other.as_ref()));
    assert_eq!(original.encode(), encoded);
    assert_eq!(original.entries().len(), MAX_NETWORK_SCAN_RESULTS);
    assert!(original.truncated());
    assert_eq!(
        serde_json::to_string(&WorkerNetworkScanSnapshot::from_scan(**original)).unwrap(),
        expected_snapshot
    );
    other.network_generation += 1;
    assert_eq!(original.network_generation, 13);
    assert_ne!(update, cloned);
}
#[test]
fn rejected_scan_preserves_retry_identity_and_never_retains_invalid_payload() {
    let mut machine = NetworkProvisioningMachine::starting_at(42).unwrap();
    let transaction = machine.request_scan().unwrap();
    let original_request = machine.next_request().unwrap().unwrap();
    let mismatched = Response {
        status: StatusCode::Ok,
        body: scan(transaction + 1).encode().to_vec(),
    };
    assert_eq!(
        machine.accept_response(&mismatched),
        Err(NetworkClientError::TransactionMismatch)
    );
    assert!(machine.latest_scan().is_none());
    assert_eq!(machine.phase(), NetworkClientPhase::Ready);
    assert_eq!(machine.next_request().unwrap().unwrap(), original_request);
    let mut invalid = scan(transaction).encode().to_vec();
    invalid[0] = 0;
    assert!(matches!(
        machine.accept_response(&Response {
            status: StatusCode::Ok,
            body: invalid
        }),
        Err(NetworkClientError::Wire(_))
    ));
    assert!(machine.latest_scan().is_none());
    assert_eq!(machine.next_request().unwrap().unwrap(), original_request);
    let accepted = machine
        .accept_response(&Response {
            status: StatusCode::Ok,
            body: scan(transaction).encode().to_vec(),
        })
        .unwrap();
    assert!(matches!(accepted, NetworkUpdate::Scan(_)));
    assert_eq!(machine.latest_scan(), Some(scan(transaction)));
}
fn summaries() -> [Option<Summary>; 7] {
    let valid = Summary {
        record_count: 8,
        realtime_record_count: 5,
        binding_count: 6,
        stepper_axes: 3,
        foc_axes: 2,
        safety_binding: true,
        flags: 0,
    };
    [
        None,
        Some(valid),
        Some(Summary {
            record_count: 0,
            ..valid
        }),
        Some(Summary {
            realtime_record_count: 9,
            ..valid
        }),
        Some(Summary {
            binding_count: 9,
            ..valid
        }),
        Some(Summary {
            stepper_axes: u8::MAX,
            foc_axes: u8::MAX,
            ..valid
        }),
        Some(Summary {
            stepper_axes: 8,
            foc_axes: 8,
            ..valid
        }),
    ]
}
#[test]
fn snapshot_identity_authorization_and_summary_truth_table() {
    let phases = [
        Phase::Empty,
        Phase::Recovering,
        Phase::Validating,
        Phase::CandidateValid,
        Phase::Preparing,
        Phase::Activating,
        Phase::Committing,
        Phase::Authorizing,
        Phase::Active,
        Phase::Clearing,
        Phase::Aborting,
        Phase::Rejected,
    ];
    let mut count = 0;
    for phase in phases {
        for jobs_authorized in [false, true] {
            for active_transaction_id in [0, 1, u64::MAX] {
                for active_digest in [[0; 32], [1; 32], std::array::from_fn(|i| u8::from(i == 31))]
                {
                    for active_bytes in [0, 1, u32::MAX] {
                        for summary in summaries() {
                            let snapshot = Snapshot {
                                phase,
                                jobs_authorized,
                                active_transaction_id,
                                active_digest,
                                active_bytes,
                                summary,
                            };
                            let parts = [
                                active_transaction_id != 0,
                                active_digest != [0; 32],
                                active_bytes != 0,
                                summary.is_some(),
                            ];
                            let present = parts.into_iter().filter(|v| *v).count();
                            let identity_ok = match present {
                                0 => !jobs_authorized && phase != Phase::Active,
                                4 => {
                                    phase != Phase::Empty
                                        && (phase != Phase::Active || jobs_authorized)
                                }
                                _ => false,
                            };
                            let summary_ok = summary.is_none_or(|s| {
                                s.record_count > 0
                                    && s.realtime_record_count <= s.record_count
                                    && s.binding_count <= s.record_count
                                    && u16::from(s.stepper_axes) + u16::from(s.foc_axes) <= 16
                            });
                            let expected = identity_ok && summary_ok;
                            let actual = snapshot.validate();
                            assert_eq!(actual.is_ok(), expected, "{snapshot:?}");
                            if !expected {
                                assert!(matches!(
                                    actual,
                                    Err(WorkerContractError::ConfigurationSnapshot)
                                ));
                            }
                            let json = serde_json::to_string(&snapshot).unwrap();
                            let decoded: Snapshot = serde_json::from_str(&json).unwrap();
                            assert_eq!(snapshot, decoded);
                            assert_eq!(decoded.validate().is_ok(), expected);
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(count, 4536);
}
