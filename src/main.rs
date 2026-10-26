#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use vsoundboard::config;

fn main() {
    let outcome = config::load(&config::config_path());
    println!("{outcome:?}");
}
