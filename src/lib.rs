pub mod agent;
pub mod app;
pub mod auth;
pub mod launch;
pub mod provider;
pub mod sys;
pub mod ui;

use nsh_core::prelude::*;

pub use ui::App;

#[native_module("piro/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}
