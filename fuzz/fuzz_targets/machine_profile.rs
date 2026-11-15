#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    alumina_interface_fuzz::machine_profile(data);
});
