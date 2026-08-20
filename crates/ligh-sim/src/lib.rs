//! iOS Simulator supervisor — headless boot, honest measure, hot sim.

mod bench;
mod device;
mod headless;
mod measure;
mod runtime;
mod simctl;
mod supervisor;

pub use runtime::{RuntimeSlim, RuntimeSlimReport};

pub use bench::{BenchReport, Benchmark};
pub use device::{DeviceManager, SimDevice};
pub use headless::{HeadlessBoot, ensure_headless};
pub use measure::{disk_available_mb, FootprintReport};
pub use simctl::Simctl;
pub use supervisor::{SimSupervisor, StatusReport};
