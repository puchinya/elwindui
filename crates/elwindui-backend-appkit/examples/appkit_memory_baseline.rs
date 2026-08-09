//! Fixed-shape AppKit baseline process used by `scripts/agent/measure-appkit-memory.sh`.

#[cfg(feature = "render-stats")]
use elwindui_backend_appkit::application;
#[cfg(feature = "render-stats")]
use elwindui_backend_appkit::diagnostics::{MemoryBaselineCase, show_memory_baseline};

#[cfg(feature = "render-stats")]
fn main() {
    let case = match std::env::args().nth(1).as_deref() {
        Some("a") | Some("A") => MemoryBaselineCase::EmptyNsView,
        Some("b") | Some("B") | Some("e") | Some("E") => MemoryBaselineCase::EmptyTreeHost,
        Some("c") | Some("C") => MemoryBaselineCase::LayerBackedTreeHost,
        _ => {
            eprintln!("usage: appkit-memory-baseline <A|B|C|E>");
            std::process::exit(2);
        }
    };
    application::run(move || show_memory_baseline(case));
}

#[cfg(not(feature = "render-stats"))]
fn main() {
    eprintln!("appkit-memory-baseline requires the render-stats feature");
}
