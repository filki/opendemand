mod engine;
pub mod error;
pub mod forecast;
pub mod types;
pub mod workspace;

#[cfg(feature = "desktop")]
mod desktop;

#[cfg(feature = "desktop")]
pub use desktop::run;
