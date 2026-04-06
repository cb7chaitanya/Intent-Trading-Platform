//! OpenTelemetry distributed tracing setup.
//!
//! Exports traces to Jaeger via OTLP (gRPC). Integrates with the
//! tracing-subscriber stack so `#[instrument]` spans and `tracing::info!`
//! calls automatically get trace_id/span_id.
//!
//! Environment variables:
//!   OTEL_EXPORTER_OTLP_ENDPOINT  — OTLP endpoint (default: http://localhost:4317)
//!   OTEL_SERVICE_NAME            — Service name (default: intentx-trading)
//!   OTEL_ENABLED                 — "true" to enable (default: false in dev)

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime,
    trace::{RandomIdGenerator, Sampler, TracerProvider},
    Resource,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize the tracing subscriber with optional OpenTelemetry export.
pub fn init(log_level: &str, environment: &str) {
    let use_json = std::env::var("LOG_FORMAT").unwrap_or_default() == "json"
        || environment == "docker"
        || environment == "production";

    let otel_enabled = std::env::var("OTEL_ENABLED")
        .unwrap_or_default()
        .eq_ignore_ascii_case("true");

    // Each branch builds the full subscriber in one shot to avoid type mismatch.
    match (otel_enabled, use_json) {
        (true, true) => {
            let provider = init_provider(environment);
            let tracer = provider.tracer("intentx");
            opentelemetry::global::set_tracer_provider(provider);

            tracing_subscriber::registry()
                .with(build_filter(log_level))
                .with(tracing_subscriber::fmt::layer().json().with_target(true))
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
        }
        (true, false) => {
            let provider = init_provider(environment);
            let tracer = provider.tracer("intentx");
            opentelemetry::global::set_tracer_provider(provider);

            tracing_subscriber::registry()
                .with(build_filter(log_level))
                .with(tracing_subscriber::fmt::layer().with_target(true))
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .init();
        }
        (false, true) => {
            tracing_subscriber::registry()
                .with(build_filter(log_level))
                .with(tracing_subscriber::fmt::layer().json().with_target(true))
                .init();
        }
        (false, false) => {
            tracing_subscriber::registry()
                .with(build_filter(log_level))
                .with(tracing_subscriber::fmt::layer().with_target(true))
                .init();
        }
    }

    if otel_enabled {
        tracing::info!("OpenTelemetry tracing enabled");
    }
}

/// Flush remaining spans and shut down the OTLP pipeline.
pub fn shutdown() {
    opentelemetry::global::shutdown_tracer_provider();
}

/// Extract the current trace ID for propagation into HTTP headers
/// or Redis messages.
pub fn current_trace_id() -> String {
    use opentelemetry::trace::TraceContextExt;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let ctx = tracing::Span::current().context();
    format!("{}", ctx.span().span_context().trace_id())
}

fn build_filter(log_level: &str) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        format!(
            "intent_trading={lvl},tower_http=info,sqlx=warn,h2=warn,hyper=warn",
            lvl = log_level
        )
        .into()
    })
}

fn init_provider(environment: &str) -> TracerProvider {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());

    let service_name = std::env::var("OTEL_SERVICE_NAME")
        .unwrap_or_else(|_| "intentx-trading".to_string());

    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&endpoint);

    opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(
            opentelemetry_sdk::trace::Config::default()
                .with_sampler(Sampler::ParentBased(Box::new(
                    Sampler::TraceIdRatioBased(sample_rate(environment)),
                )))
                .with_id_generator(RandomIdGenerator::default())
                .with_resource(Resource::new(vec![
                    KeyValue::new("service.name", service_name),
                    KeyValue::new("deployment.environment", environment.to_string()),
                    KeyValue::new("service.version", env!("CARGO_PKG_VERSION").to_string()),
                ])),
        )
        .install_batch(runtime::Tokio)
        .expect("Failed to install OTLP tracing pipeline")
}

fn sample_rate(environment: &str) -> f64 {
    match environment {
        "production" => 0.1,
        "docker" => 0.5,
        _ => 1.0,
    }
}
