#![forbid(unsafe_code)]
#![doc = "Shared Dioxus application and action model for OMP Remote."]

mod actions;
mod model;
mod platform;
mod profiles;
mod qr;
mod view;

pub use actions::*;
pub use model::*;
pub use platform::*;
pub use profiles::*;
pub use qr::*;
pub use view::app;
