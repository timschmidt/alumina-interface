//! Emits a strict simulator-targeted cached-job request for browser transport testing.

use alumina_interface::{CachedJobDeploymentTarget, compile_representative_cached_job_request};
use alumina_protocol::DeviceId;
use alumina_storage::sha256;

const DEFAULT_JOB_ID: u64 = 0x7a11_0001;

fn main() -> Result<(), String> {
    let mut device_ids = Vec::new();
    let mut job_id = DEFAULT_JOB_ID;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let value = arguments
            .next()
            .ok_or_else(|| format!("{argument} requires a value"))?;
        match argument.as_str() {
            "--device-id" => device_ids.push(parse_device_id(&value)?),
            "--job-id" => {
                job_id = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value != 0)
                    .ok_or_else(|| "--job-id must be a nonzero u64".to_owned())?;
            }
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    if device_ids.is_empty() {
        device_ids.push(DeviceId(*b"ALUM-SIM:TINYBEE"));
    }
    device_ids.sort_unstable();
    if device_ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("--device-id values must be unique".to_owned());
    }
    let configuration_bytes =
        alumina_sim::configuration::representative_tinybee_configuration_bytes(
            alumina_sim::capability::CAPABILITY_DIGEST,
        )
        .map_err(|error| format!("simulator configuration rejected: {error:?}"))?;
    let config_digest = sha256(&configuration_bytes).digest;
    let targets: Vec<_> = device_ids
        .into_iter()
        .enumerate()
        .map(|(index, device_id)| {
            let connection_id = u64::try_from(index)
                .map_err(|_| "too many simulator targets".to_owned())?
                .checked_add(1)
                .ok_or_else(|| "simulator connection identity overflowed".to_owned())?;
            Ok(CachedJobDeploymentTarget {
                connection_id,
                // The harness configures each new connection exactly once in this order.
                generation: connection_id,
                device_id,
                boot_id: [0x31; 16],
                capability_digest: alumina_sim::capability::CAPABILITY_DIGEST,
                config_digest,
            })
        })
        .collect::<Result<_, String>>()?;
    let request = compile_representative_cached_job_request(job_id, &targets)?;
    println!(
        "{}",
        serde_json::to_string(&request)
            .map_err(|error| format!("worker request serialization failed: {error}"))?
    );
    Ok(())
}

fn parse_device_id(value: &str) -> Result<DeviceId, String> {
    if value.len() != 32 {
        return Err("--device-id must contain exactly 32 hexadecimal digits".to_owned());
    }
    let mut bytes = [0_u8; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hexadecimal_nibble(pair[0])
            .ok_or_else(|| "--device-id contains a non-hexadecimal digit".to_owned())?;
        let low = hexadecimal_nibble(pair[1])
            .ok_or_else(|| "--device-id contains a non-hexadecimal digit".to_owned())?;
        bytes[index] = (high << 4) | low;
    }
    let device = DeviceId(bytes);
    if device.is_zero() {
        Err("--device-id may not use the all-zero sentinel".to_owned())
    } else {
        Ok(device)
    }
}

const fn hexadecimal_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
