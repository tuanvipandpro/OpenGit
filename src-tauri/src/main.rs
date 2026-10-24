#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if let Some(code) = opengit_lib::auth::helper_entry() {
        std::process::exit(code);
    }
    opengit_lib::run();
}
