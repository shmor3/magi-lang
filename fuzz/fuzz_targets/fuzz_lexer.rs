#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Convert arbitrary bytes to a UTF-8 string, skipping invalid inputs.
    if let Ok(source) = std::str::from_utf8(data) {
        // Feed the string to the lexer. Errors are fine; panics are not.
        let _ = magi_lang::syntax::lexer::tokenize(source);
    }
});
