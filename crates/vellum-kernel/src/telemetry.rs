// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Traces, metrics and logs from the first commit.
//!
//! Every Vellum process calls [`init`] once at start-up. Logs always go to
//! standard error, as JSON by default. When `OTEL_EXPORTER_OTLP_ENDPOINT` is
//! set, traces, metrics and logs are also exported over OTLP/HTTP to that
//! collector, and `tracing` spans become OpenTelemetry spans carrying the
//! W3C trace context.
//!
//! | Variable | Meaning | Default |
//! |---|---|---|
//! | `VELLUM_LOG` | `tracing` filter directives | `info` |
//! | `VELLUM_LOG_FORMAT` | `json` or `pretty` | `json` |
//! | `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP/HTTP collector base URL | unset (no export) |

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{
    LogExporter, MetricExporter, SpanExporter, WithExportConfig as _,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::{MetricsLayer, OpenTelemetryLayer};
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::{EnvFilter, Layer as _, Registry};

/// How log lines are written to standard error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// One JSON object per line, for machines and log shippers.
    #[default]
    Json,
    /// Human-readable lines, for local development.
    Pretty,
}

/// Settings for [`init`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryConfig {
    /// The `service.name` resource attribute, for example `vellum`.
    pub service_name: String,
    /// The service version, normally `env!("CARGO_PKG_VERSION")`.
    pub service_version: String,
    /// `tracing` filter directives, such as `info,vellum_db=debug`.
    pub filter: String,
    /// How log lines are written to standard error.
    pub log_format: LogFormat,
    /// OTLP/HTTP collector base URL; `None` disables export.
    pub otlp_endpoint: Option<String>,
}

impl TelemetryConfig {
    /// Builds a configuration from the process environment.
    #[must_use]
    pub fn from_env(service_name: &str, service_version: &str) -> Self {
        Self::from_lookup(service_name, service_version, |key| {
            std::env::var(key).ok()
        })
    }

    /// Builds a configuration from any key lookup; [`Self::from_env`] uses
    /// the process environment.
    #[must_use]
    pub fn from_lookup(
        service_name: &str,
        service_version: &str,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let non_empty =
            |key: &str| lookup(key).filter(|value| !value.trim().is_empty());
        let log_format = match non_empty("VELLUM_LOG_FORMAT").as_deref() {
            Some("pretty") => LogFormat::Pretty,
            _ => LogFormat::Json,
        };
        Self {
            service_name: service_name.to_owned(),
            service_version: service_version.to_owned(),
            filter: non_empty("VELLUM_LOG")
                .unwrap_or_else(|| "info".to_owned()),
            log_format,
            otlp_endpoint: non_empty("OTEL_EXPORTER_OTLP_ENDPOINT"),
        }
    }
}

/// Why telemetry could not start.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// `VELLUM_LOG` is not a valid filter.
    #[error("invalid log filter: {0}")]
    Filter(#[from] tracing_subscriber::filter::ParseError),
    /// An OTLP exporter could not be built.
    #[error("cannot build the OTLP exporter: {0}")]
    Exporter(#[from] opentelemetry_otlp::ExporterBuildError),
    /// A global subscriber was already installed.
    #[error("telemetry is already initialised: {0}")]
    AlreadyInitialised(#[from] tracing::subscriber::SetGlobalDefaultError),
}

/// Keeps the exporters alive; dropping it flushes and shuts them down.
#[derive(Debug, Default)]
#[must_use = "dropping the guard shuts telemetry down"]
pub struct TelemetryGuard {
    providers: Option<Providers>,
}

impl TelemetryGuard {
    /// Whether traces, metrics and logs are exported over OTLP.
    #[must_use]
    pub const fn is_exporting(&self) -> bool {
        self.providers.is_some()
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(providers) = self.providers.take() {
            // Shutdown errors cannot be reported anywhere useful once the
            // pipeline itself is gone, so they are ignored by design.
            let _ = providers.tracer.shutdown();
            let _ = providers.meter.shutdown();
            let _ = providers.logger.shutdown();
        }
    }
}

#[derive(Debug)]
struct Providers {
    tracer: SdkTracerProvider,
    meter: SdkMeterProvider,
    logger: SdkLoggerProvider,
}

impl Providers {
    fn build(
        config: &TelemetryConfig,
        endpoint: &str,
    ) -> Result<Self, TelemetryError> {
        let base = endpoint.trim_end_matches('/');
        let resource = Resource::builder()
            .with_service_name(config.service_name.clone())
            .with_attribute(opentelemetry::KeyValue::new(
                "service.version",
                config.service_version.clone(),
            ))
            .build();
        let spans = SpanExporter::builder()
            .with_http()
            .with_endpoint(format!("{base}/v1/traces"))
            .build()?;
        let metrics = MetricExporter::builder()
            .with_http()
            .with_endpoint(format!("{base}/v1/metrics"))
            .build()?;
        let logs = LogExporter::builder()
            .with_http()
            .with_endpoint(format!("{base}/v1/logs"))
            .build()?;
        Ok(Self {
            tracer: SdkTracerProvider::builder()
                .with_resource(resource.clone())
                .with_batch_exporter(spans)
                .build(),
            meter: SdkMeterProvider::builder()
                .with_resource(resource.clone())
                .with_periodic_exporter(metrics)
                .build(),
            logger: SdkLoggerProvider::builder()
                .with_resource(resource)
                .with_batch_exporter(logs)
                .build(),
        })
    }
}

/// Builds the subscriber described by `config` without installing it.
///
/// [`init`] installs it globally; tests use it with
/// [`tracing::subscriber::with_default`].
///
/// # Errors
///
/// Returns [`TelemetryError`] when the filter is invalid or an exporter
/// cannot be built.
pub fn subscriber(
    config: &TelemetryConfig,
) -> Result<
    (
        impl tracing::Subscriber + Send + Sync + use<>,
        TelemetryGuard,
    ),
    TelemetryError,
> {
    subscriber_with(config, None)
}

/// A layer added beneath the kernel's own, such as the Admin → Requests
/// recorder from `vellum-observe`.
pub type ExtraLayer =
    Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync>;

/// Like [`subscriber`], with one extra layer that sees every span and
/// event the filter lets through.
///
/// # Errors
///
/// Returns [`TelemetryError`] when the filter is invalid or an exporter
/// cannot be built.
pub fn subscriber_with(
    config: &TelemetryConfig,
    extra: Option<ExtraLayer>,
) -> Result<
    (
        impl tracing::Subscriber + Send + Sync + use<>,
        TelemetryGuard,
    ),
    TelemetryError,
> {
    let filter = EnvFilter::try_new(&config.filter)?;
    let stderr = match config.log_format {
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(false)
            .with_writer(std::io::stderr)
            .boxed(),
        LogFormat::Pretty => tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .boxed(),
    };
    let providers = config
        .otlp_endpoint
        .as_deref()
        .map(|endpoint| Providers::build(config, endpoint))
        .transpose()?;
    let otel = providers.as_ref().map(|p| {
        let tracer = p.tracer.tracer("vellum");
        OpenTelemetryLayer::new(tracer)
            .and_then(MetricsLayer::new(p.meter.clone()))
            .and_then(OpenTelemetryTracingBridge::new(&p.logger).with_filter(
                // The exporters log through `tracing` too; keep their own
                // events out of the export pipeline to avoid a feedback loop.
                filter_fn(|meta| !is_exporter_target(meta.target())),
            ))
    });
    let subscriber = Registry::default()
        .with(extra)
        .with(filter)
        .with(stderr)
        .with(otel);
    if let Some(p) = &providers {
        opentelemetry::global::set_meter_provider(p.meter.clone());
        opentelemetry::global::set_tracer_provider(p.tracer.clone());
    }
    Ok((subscriber, TelemetryGuard { providers }))
}

/// The meter for `scope` on the process's meter provider.
///
/// Instruments made from it export over OTLP once [`init`] installed an
/// exporter, and cost nothing otherwise. Observable gauges registered on it are
/// read at each export, so their callbacks should only read cached values.
#[must_use]
pub fn meter(scope: &'static str) -> opentelemetry::metrics::Meter {
    opentelemetry::global::meter(scope)
}

/// Whether a `tracing` target belongs to the export pipeline itself.
fn is_exporter_target(target: &str) -> bool {
    ["opentelemetry", "hyper", "reqwest", "h2", "rustls"]
        .iter()
        .any(|prefix| target.starts_with(prefix))
}

/// Installs telemetry for this process. Call once, early in `main`, and
/// keep the returned guard alive until the process exits.
///
/// # Errors
///
/// Returns [`TelemetryError`] when the configuration is invalid or
/// telemetry was already initialised.
pub fn init(
    config: &TelemetryConfig,
) -> Result<TelemetryGuard, TelemetryError> {
    init_with(config, None)
}

/// Like [`init`], with one extra layer; see [`subscriber_with`].
///
/// # Errors
///
/// Returns [`TelemetryError`] when the configuration is invalid or
/// telemetry was already initialised.
pub fn init_with(
    config: &TelemetryConfig,
    extra: Option<ExtraLayer>,
) -> Result<TelemetryGuard, TelemetryError> {
    let (subscriber, guard) = subscriber_with(config, extra)?;
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(pairs: &[(&str, &str)]) -> TelemetryConfig {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        TelemetryConfig::from_lookup("vellum-test", "0.0.0", |key| {
            env.get(key).cloned()
        })
    }

    #[test]
    fn defaults_are_json_info_and_no_export() {
        let c = config(&[]);
        assert_eq!(c.service_name, "vellum-test");
        assert_eq!(c.service_version, "0.0.0");
        assert_eq!(c.filter, "info");
        assert_eq!(c.log_format, LogFormat::Json);
        assert_eq!(c.otlp_endpoint, None);
    }

    #[test]
    fn reads_every_variable() {
        let c = config(&[
            ("VELLUM_LOG", "debug,hyper=warn"),
            ("VELLUM_LOG_FORMAT", "pretty"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318"),
        ]);
        assert_eq!(c.filter, "debug,hyper=warn");
        assert_eq!(c.log_format, LogFormat::Pretty);
        assert_eq!(c.otlp_endpoint.as_deref(), Some("http://collector:4318"));
    }

    #[test]
    fn blank_values_fall_back_to_defaults() {
        let c =
            config(&[("VELLUM_LOG", " "), ("OTEL_EXPORTER_OTLP_ENDPOINT", "")]);
        assert_eq!(c.filter, "info");
        assert_eq!(c.otlp_endpoint, None);
    }

    #[test]
    fn unknown_format_means_json() {
        assert_eq!(
            config(&[("VELLUM_LOG_FORMAT", "xml")]).log_format,
            LogFormat::Json
        );
    }

    #[test]
    fn invalid_filter_is_an_error() {
        let result = subscriber(&config(&[("VELLUM_LOG", "info,[")]));
        assert!(matches!(result, Err(TelemetryError::Filter(_))));
    }

    #[test]
    fn local_only_subscriber_records_spans() {
        for format in ["json", "pretty"] {
            let (sub, guard) =
                subscriber(&config(&[("VELLUM_LOG_FORMAT", format)])).unwrap();
            assert!(!guard.is_exporting());
            tracing::subscriber::with_default(sub, || {
                let span = tracing::info_span!("kernel.test");
                assert!(!span.is_disabled());
                tracing::info!(parent: &span, "hello");
            });
        }
    }

    #[test]
    fn exporting_subscriber_builds_and_shuts_down() {
        let c =
            config(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:9/")]);
        let (sub, guard) = subscriber(&c).unwrap();
        assert!(guard.is_exporting());
        tracing::subscriber::with_default(sub, || {
            let span = tracing::info_span!("kernel.export");
            let _entered = span.enter();
            tracing::info!(
                monotonic_counter.kernel_test_events = 1_u64,
                "exported"
            );
        });
        drop(guard);
    }

    #[test]
    fn extra_layer_sees_spans() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Count(Arc<AtomicUsize>);
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Count {
            fn on_new_span(
                &self,
                _: &tracing::span::Attributes<'_>,
                _: &tracing::span::Id,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                let _ = self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let seen = Arc::new(AtomicUsize::new(0));
        let (sub, _guard) = subscriber_with(
            &config(&[]),
            Some(Box::new(Count(Arc::clone(&seen)))),
        )
        .unwrap();
        tracing::subscriber::with_default(sub, || {
            let span = tracing::info_span!("kernel.extra");
            drop(span.enter());
        });
        assert_eq!(seen.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn exporter_targets_are_not_re_exported() {
        assert!(is_exporter_target("opentelemetry_sdk"));
        assert!(is_exporter_target("hyper::client"));
        assert!(!is_exporter_target("vellum_kernel"));
    }

    #[test]
    fn init_installs_once() {
        let guard = init(&config(&[])).unwrap();
        assert!(!guard.is_exporting());
        assert!(matches!(
            init(&config(&[])),
            Err(TelemetryError::AlreadyInitialised(_))
        ));
    }
}
