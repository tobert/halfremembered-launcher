use anyhow::{Context, Result};
use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider, Severity};
use opentelemetry::{Key, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::logs::{Logger as SdkLogger, LoggerProvider as SdkLoggerProvider};
use opentelemetry_sdk::Resource;
use std::time::SystemTime;

/// OTLP log exporter for forwarding process logs
pub struct OtlpExporter {
    logger: SdkLogger,
    _provider: SdkLoggerProvider, // Keep alive for cleanup
}

impl OtlpExporter {
    /// Initialize OTLP exporter with the given endpoint
    pub fn new(endpoint: &str) -> Result<Self> {
        log::info!("Initializing OTLP exporter: {}", endpoint);

        // Create resource with service identification
        let resource = Resource::new(vec![
            KeyValue::new("service.name", "halfremembered-launcher"),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        ]);

        // Build OTLP exporter with gRPC transport
        let exporter = opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(endpoint)
            .build_log_exporter()
            .context("Failed to build OTLP log exporter")?;

        // Create logger provider with the exporter
        let provider = SdkLoggerProvider::builder()
            .with_resource(resource)
            .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
            .build();

        let logger = provider.logger("halfremembered-launcher");

        log::info!("✅ OTLP exporter initialized successfully");

        Ok(Self {
            logger,
            _provider: provider,
        })
    }

    /// Send a log line to the OTLP endpoint
    pub fn send_log(
        &self,
        request_id: &str,
        hostname: &str,
        binary: &str,
        stream: &str, // "stdout" or "stderr"
        line: &str,
    ) {
        let severity = if stream == "stderr" {
            Severity::Warn
        } else {
            Severity::Info
        };

        let mut record = self.logger.create_log_record();
        record.set_timestamp(SystemTime::now());
        record.set_severity_number(severity);
        record.set_body(AnyValue::from(line.to_string()));
        record.add_attribute(
            Key::from("request_id"),
            AnyValue::from(request_id.to_string()),
        );
        record.add_attribute(
            Key::from("client.hostname"),
            AnyValue::from(hostname.to_string()),
        );
        record.add_attribute(
            Key::from("process.binary"),
            AnyValue::from(binary.to_string()),
        );
        record.add_attribute(
            Key::from("stream.type"),
            AnyValue::from(stream.to_string()),
        );

        self.logger.emit(record);
    }
}

// Ensure logs are flushed on drop
impl Drop for OtlpExporter {
    fn drop(&mut self) {
        log::debug!("Shutting down OTLP exporter");
    }
}

#[cfg(test)]
mod tests {
    // Note: These tests require a running OTLP endpoint to fully validate.
    // Run manual tests with: ./test-otlp.sh
    //
    // TODO: Add integration tests that spin up a mock OTLP collector
}

