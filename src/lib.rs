mod app;
mod config;
mod api;
mod components;
pub mod format;
mod models;
mod pages;
mod pricing;

use app::App;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
