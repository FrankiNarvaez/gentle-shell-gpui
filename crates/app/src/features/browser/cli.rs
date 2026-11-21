//! `gentle-browser`: how agents drive pages in the app's browser. The shim
//! runs `gentle-shell-gpui --browser …`; this process hands the operation to
//! the running app and prints its JSON answer.

use gentle_core::browser::parse_args;

use crate::platform::instance;

/// `gentle-shell-gpui --browser <args>`: the exit code (1 for an error the
/// app answered, 2 for bad arguments, 124 for a timeout).
pub fn run(args: Vec<String>) -> i32 {
    let op = match parse_args(&args) {
        Ok(op) => op,
        Err(usage) => {
            eprintln!("{usage}");
            return 2;
        }
    };
    let Ok(value) = serde_json::to_value(&op) else {
        return 2;
    };
    let wait = gentle_core::browser::answer_within(&value);
    let answer = match instance::browser(&value, wait) {
        Ok(answer) => answer,
        Err(message) => {
            eprintln!("gentle-browser: {message}");
            return 1;
        }
    };
    println!("{answer}");
    match answer["error"]["kind"].as_str() {
        None => 0,
        Some("timeout") => 124,
        Some(_) => 1,
    }
}
