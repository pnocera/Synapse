use super::*;
use std::{
    collections::BTreeMap,
    error::Error,
    sync::{Arc, Mutex},
    time::Duration,
};

use metrics::{
    Counter, CounterFn, Gauge, GaugeFn, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
    SharedString, Unit,
};

const TEST_SCHEMA_VERSION: u32 = 7;
const METRIC_KEY: &str = "cache_evictions_total{cf=CF_EVENTS,reason=soft_cap}";

#[test]
fn gc_soft_cap_hard_cap_edges_and_metrics_with_fsv() -> Result<(), Box<dyn Error>> {
    let recorder = TestRecorder::default();
    metrics::with_local_recorder(&recorder, || -> Result<(), Box<dyn Error>> {
        run_gc_case(
            &recorder,
            CaseSpec::new("below_soft", 9, 10, 20, 9, 0, false),
        )?;
        run_gc_case(
            &recorder,
            CaseSpec::new("at_soft", 10, 10, 20, 10, 0, false),
        )?;
        run_gc_case(
            &recorder,
            CaseSpec::new("soft_cap", 20, 10, 30, 10, 10, false),
        )?;
        run_gc_case(
            &recorder,
            CaseSpec::new("hard_cap", 25, 10, 20, 10, 15, true),
        )?;
        Ok(())
    })?;
    Ok(())
}

#[tokio::test]
async fn gc_periodic_task_runs_tick_with_fsv() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?;
    fill_rows(&db, 12)?;
    db.flush()?;
    let before = db.scan_cf(cf::CF_EVENTS)?;
    let config = gc::GcConfig::rows(Duration::from_millis(10), cf::CF_EVENTS, 6, 20);
    let task = gc::spawn(Arc::clone(&db.inner), config)?;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let after = db.scan_cf(cf::CF_EVENTS)?;
    println!(
        "source_of_truth=cf_scan case=periodic_task before_count={} after_count={} final_value=spawned_tick_evicted:{}",
        before.len(),
        after.len(),
        before.len().saturating_sub(after.len())
    );
    drop(task);
    assert!(after.len() <= 6);
    Ok(())
}

#[derive(Clone, Copy)]
struct CaseSpec {
    name: &'static str,
    rows: usize,
    soft_cap: u64,
    hard_cap: u64,
    expected_after: usize,
    expected_evicted: u64,
    expect_hard_cap: bool,
}

impl CaseSpec {
    const fn new(
        name: &'static str,
        rows: usize,
        soft_cap: u64,
        hard_cap: u64,
        expected_after: usize,
        expected_evicted: u64,
        expect_hard_cap: bool,
    ) -> Self {
        Self {
            name,
            rows,
            soft_cap,
            hard_cap,
            expected_after,
            expected_evicted,
            expect_hard_cap,
        }
    }
}

fn run_gc_case(recorder: &TestRecorder, case: CaseSpec) -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?;
    fill_rows(&db, case.rows)?;
    db.flush()?;
    let before = db.scan_cf(cf::CF_EVENTS)?;
    let before_property = estimated_num_keys(&db)?;
    let before_metric = recorder.counter_value(METRIC_KEY)?;
    let config = gc::GcConfig::rows(
        Duration::from_mins(5),
        cf::CF_EVENTS,
        case.soft_cap,
        case.hard_cap,
    );

    let report = gc::run_once(&db.inner, &config)?;
    let cf_report = report
        .cf(cf::CF_EVENTS)
        .ok_or("GC report missing CF_EVENTS")?;
    let after = db.scan_cf(cf::CF_EVENTS)?;
    let after_property = estimated_num_keys(&db)?;
    let after_metric = recorder.counter_value(METRIC_KEY)?;
    println!(
        "source_of_truth=cf_scan case={} before_count={} before_property={before_property:?} after_count={} after_property={after_property:?} evicted={} metric_delta={} hard_cap_code={:?} final_value=rows:{}",
        case.name,
        before.len(),
        after.len(),
        cf_report.evicted_rows,
        after_metric.saturating_sub(before_metric),
        cf_report.hard_cap_code,
        printable_keys(&after)
    );

    assert_eq!(after.len(), case.expected_after);
    assert_eq!(cf_report.evicted_rows, case.expected_evicted);
    assert_eq!(
        after_metric.saturating_sub(before_metric),
        case.expected_evicted
    );
    assert_eq!(cf_report.hard_cap_reached, case.expect_hard_cap);
    assert_eq!(
        cf_report.hard_cap_code,
        case.expect_hard_cap
            .then_some(synapse_core::error_codes::STORAGE_CF_HARD_CAP_REACHED)
    );
    let property = after_property.ok_or("rocksdb.estimate-num-keys returned None")?;
    assert!(property <= case.expected_after as u64);
    if case.expected_evicted > 0 {
        let first_key = after.first().ok_or("GC removed every row unexpectedly")?;
        assert_eq!(
            String::from_utf8_lossy(&first_key.0),
            format!("{:016x}", case.expected_evicted)
        );
    }
    Ok(())
}

fn fill_rows(db: &Db, rows: usize) -> StorageResult<()> {
    let kvs = (0..rows)
        .map(|index| {
            (
                format!("{index:016x}").into_bytes(),
                format!(r#"{{"label":"gc","seq":{index}}}"#).into_bytes(),
            )
        })
        .collect::<Vec<_>>();
    db.put_batch(cf::CF_EVENTS, kvs)
}

fn estimated_num_keys(db: &Db) -> StorageResult<Option<u64>> {
    let cf = db
        .inner
        .cf_handle(cf::CF_EVENTS)
        .ok_or_else(|| StorageError::ReadFailed {
            cf_name: cf::CF_EVENTS.to_owned(),
            detail: "column family handle missing".to_owned(),
        })?;
    db.inner
        .property_int_value_cf(&cf, "rocksdb.estimate-num-keys")
        .map_err(|error| StorageError::ReadFailed {
            cf_name: cf::CF_EVENTS.to_owned(),
            detail: error.to_string(),
        })
}

fn printable_keys(rows: &[(Vec<u8>, Vec<u8>)]) -> String {
    rows.iter()
        .map(|(key, _value)| String::from_utf8_lossy(key).into_owned())
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Clone, Default)]
struct TestRecorder {
    counters: Arc<Mutex<BTreeMap<String, u64>>>,
}

impl TestRecorder {
    fn counter_value(&self, key: &str) -> Result<u64, Box<dyn Error>> {
        let counters = self
            .counters
            .lock()
            .map_err(|error| format!("metric recorder lock poisoned: {error}"))?;
        Ok(counters.get(key).copied().unwrap_or_default())
    }
}

impl Recorder for TestRecorder {
    fn describe_counter(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn describe_gauge(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn describe_histogram(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> Counter {
        Counter::from_arc(Arc::new(TestCounter {
            key: metric_key(key),
            counters: Arc::clone(&self.counters),
        }))
    }

    fn register_gauge(&self, _key: &Key, _metadata: &Metadata<'_>) -> Gauge {
        Gauge::from_arc(Arc::new(NoopGauge))
    }

    fn register_histogram(&self, _key: &Key, _metadata: &Metadata<'_>) -> Histogram {
        Histogram::from_arc(Arc::new(NoopHistogram))
    }
}

struct TestCounter {
    key: String,
    counters: Arc<Mutex<BTreeMap<String, u64>>>,
}

impl CounterFn for TestCounter {
    fn increment(&self, value: u64) {
        if let Ok(mut counters) = self.counters.lock() {
            let counter = counters.entry(self.key.clone()).or_default();
            *counter = counter.saturating_add(value);
        }
    }

    fn absolute(&self, value: u64) {
        if let Ok(mut counters) = self.counters.lock() {
            counters.insert(self.key.clone(), value);
        }
    }
}

struct NoopGauge;

impl GaugeFn for NoopGauge {
    fn increment(&self, _value: f64) {}

    fn decrement(&self, _value: f64) {}

    fn set(&self, _value: f64) {}
}

struct NoopHistogram;

impl HistogramFn for NoopHistogram {
    fn record(&self, _value: f64) {}
}

fn metric_key(key: &Key) -> String {
    let mut labels = key
        .labels()
        .map(|label| format!("{}={}", label.key(), label.value()))
        .collect::<Vec<_>>();
    labels.sort();
    format!("{}{{{}}}", key.name(), labels.join(","))
}
