#[cfg(feature = "timing")]
use std::time::Duration;
use std::time::Instant;

pub use rift_protocol::MetricsCommand;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry};
#[cfg(feature = "timing")]
use tracing_timing::{Histogram, group};
use tracing_tree::time::UtcDateTime;

pub fn init_logging() {
    // Keep timing independent of output verbosity so profiling can run without
    // synchronous trace output. Do not construct recorder storage in normal use.
    let subscriber = tracing_subscriber::registry()
        .with(tree_layer().with_filter(EnvFilter::from_default_env()));
    #[cfg(feature = "timing")]
    let subscriber = subscriber.with(timing_layer().with_filter(EnvFilter::new("trace")));
    subscriber.init();
}

pub fn tree_layer() -> impl Layer<Registry> {
    tracing_tree::HierarchicalLayer::default()
        .with_indent_amount(2)
        .with_indent_lines(true)
        .with_deferred_spans(true)
        .with_span_retrace(true)
        .with_targets(true)
        .with_timer(UtcDateTime::default())
}

#[cfg(feature = "timing")]
type TimingLayer = tracing_timing::TimingLayer<group::ByName, group::ByName>;

#[cfg(feature = "timing")]
fn timing_layer() -> TimingLayer {
    tracing_timing::Builder::default()
        // Formatted messages contain changing window IDs, frames, and pointer values.
        // Histograms persist for the process lifetime, so key them by the fixed call site.
        .events(group::ByName)
        .layer(|| Histogram::new_with_max(100_000_000, 2).unwrap())
}

pub fn handle_command(command: MetricsCommand) {
    match command {
        MetricsCommand::ShowTiming => show_timing(),
    }
}

pub fn show_timing() {
    show_layout_core();
    show_roc_core();
    #[cfg(feature = "timing")]
    tracing::dispatcher::get_default(|d| {
        if let Some(timing_layer) = d.downcast_ref::<TimingLayer>() {
            print_histograms(timing_layer);
        } else {
            println!("Timing collection is unavailable in the current tracing subscriber.");
        }
    });
    #[cfg(not(feature = "timing"))]
    println!(
        "Timing collection is disabled. Build rift with --features timing to collect metrics."
    );
}

/// The core deciding layouts (policy (a)): how often a shadow's frames (a layout system's, or
/// under `roc-core` the Rust core's) differed from the deciding core's, each one a
/// `layout-divergence:` or `roc-divergence:`, out of how many times a layout system's shadow ran
/// (every layout pass in tests and `roc-core` builds, every 32nd pass per Space otherwise).
fn show_layout_core() {
    let divergences = crate::layout_engine::layout_divergences();
    let shadows = crate::layout_engine::layout_shadows();
    println!("core: layout divergences: {divergences} (shadows run: {shadows})");
    tracing::info!(divergences, shadows, "core: layout divergences");
    // The core's floating system beside upstream's (each difference a `floating-divergence:`).
    let divergences = crate::layout_engine::systems::floating_divergences();
    println!("core: floating divergences: {divergences}");
    tracing::info!(divergences, "core: floating divergences");
}

/// The Roc core's standing on rift's path: how often a flipped machine's Rust shadow, or a Roc
/// shadow (the BSP tree), had to be re-seeded (each one a `roc-divergence:`). Only with
/// `roc-core`.
fn show_roc_core() {
    #[cfg(feature = "roc-core")]
    {
        let reseeds = crate::roc_twin::lifecycle_reseeds();
        println!("roc-core: lifecycle re-seeds: {reseeds}");
        tracing::info!(reseeds, "roc-core: lifecycle re-seeds");
        let reseeds = crate::roc_twin::main_window_reseeds();
        println!("roc-core: main-window re-seeds: {reseeds}");
        tracing::info!(reseeds, "roc-core: main-window re-seeds");
        let reseeds = crate::roc_twin::workspaces_reseeds();
        println!("roc-core: workspaces re-seeds: {reseeds}");
        tracing::info!(reseeds, "roc-core: workspaces re-seeds");
        // The Rust BSP tree, the deciding Roc tree's shadow (re-seeded from it).
        let reseeds = crate::layout_engine::systems::bsp_tree_reseeds();
        println!("roc-core: bsp tree re-seeds: {reseeds}");
        tracing::info!(reseeds, "roc-core: bsp tree re-seeds");
        // The Rust stacks, the deciding Roc stacks' shadow (re-seeded from them).
        let reseeds = crate::layout_engine::systems::stack_tree_reseeds();
        println!("roc-core: stack tree re-seeds: {reseeds}");
        tracing::info!(reseeds, "roc-core: stack tree re-seeds");
        // The Roc group forest, the core's second shadow (re-seeded from the core).
        let reseeds = crate::layout_engine::systems::group_forest_reseeds();
        println!("roc-core: group forest re-seeds: {reseeds}");
        tracing::info!(reseeds, "roc-core: group forest re-seeds");
    }
}

#[cfg(feature = "timing")]
fn print_histograms(timing_layer: &TimingLayer) {
    timing_layer.force_synchronize();
    timing_layer.with_histograms(|hs| {
        println!("\nHistograms:\n");
        for (span, hs) in hs {
            for (event, h) in hs {
                let ns = |nanos| Duration::from_nanos(nanos);
                println!("{span} -> {event} ({} events)", h.len());
                println!("    mean: {:?}", ns(h.mean() as u64));
                println!("    min: {:?}", ns(h.min()));
                println!("    p50: {:?}", ns(h.value_at_quantile(0.50)));
                println!("    p90: {:?}", ns(h.value_at_quantile(0.90)));
                println!("    p99: {:?}", ns(h.value_at_quantile(0.99)));
                println!("    max: {:?}", ns(h.max()));
            }
        }
        println!();
    });
}

pub fn trace_misc<T>(desc: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    let end = Instant::now();
    tracing::trace!(time = ?(end - start), "{desc}");
    out
}
