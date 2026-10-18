#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    lightfoundry_fuzzkit::ai_reply(data);
});
