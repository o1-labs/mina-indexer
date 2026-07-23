//! GraphQL `timeLocks` endpoint.
//!
//! Rolls up the network's locked/vesting supply schedule (the same embedded
//! `data/locked.csv` the REST `blockchain_summary` uses for `locked_supply`)
//! into day / month / year / all-time buckets. This is the indexer-side source
//! for Blockberry's `getTimeLocksAll` / `Day` / `Month` / `Year` (issue #95
//! item 4); a gateway maps each endpoint to a `bucket` value.
//!
//! The CSV is a static, monotonically non-increasing schedule (locked supply
//! only unlocks over time), so the rollups are derived once, process-wide, via
//! [`LazyLock`] -- no per-query parse, and queries that never touch `timeLocks`
//! never pay for it.

use crate::web::rest::locked_balances::LOCKED_BALANCES_CONTENTS;
use async_graphql::{Enum, Object, Result, SimpleObject};
use std::sync::LazyLock;

/// Bucket granularity for the locked-supply rollup.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum TimeLockBucket {
    /// A single all-time bucket (the whole schedule).
    All,
    Day,
    Month,
    Year,
}

#[derive(SimpleObject)]
pub struct TimeLockEntry {
    /// Bucket label: `all`, `YYYY`, `YYYY-MM`, or `YYYY-MM-DD` (UTC).
    pub date: String,

    /// Last global slot in the bucket (the slot the bucket's state is reported
    /// as of).
    #[graphql(name = "global_slot")]
    pub global_slot: u32,

    /// Locked supply at the end of the bucket, in whole MINA.
    #[graphql(name = "locked_supply")]
    pub locked_supply: u64,

    /// Supply unlocked (vested) during the bucket, in whole MINA -- the drop in
    /// locked supply from the previous bucket's close to this one's.
    pub unlocked: u64,
}

/// One parsed CSV row: `(global slot, locked MINA, ISO-8601 UTC datetime)`.
struct ScheduleRow {
    slot: u32,
    locked: u64,
    datetime: String,
}

/// Plain (non-GraphQL) rollup row, cached in [`ROLLUPS`].
#[derive(Clone)]
struct RollupRow {
    date: String,
    global_slot: u32,
    locked_supply: u64,
    unlocked: u64,
}

impl From<&RollupRow> for TimeLockEntry {
    fn from(r: &RollupRow) -> Self {
        Self {
            date: r.date.clone(),
            global_slot: r.global_slot,
            locked_supply: r.locked_supply,
            unlocked: r.unlocked,
        }
    }
}

struct Rollups {
    all: Vec<RollupRow>,
    day: Vec<RollupRow>,
    month: Vec<RollupRow>,
    year: Vec<RollupRow>,
}

/// Parse the embedded schedule and fold it into every bucket granularity, once.
/// The raw per-slot rows (~700k) are dropped after the fold; only the small
/// rollups (≤ a few thousand rows total) persist.
static ROLLUPS: LazyLock<Rollups> = LazyLock::new(|| {
    let mut rows = Vec::new();
    let mut rdr = csv::Reader::from_reader(LOCKED_BALANCES_CONTENTS.as_bytes());
    for record in rdr.records().flatten() {
        // columns: slot, locked (whole MINA), datetime (ISO-8601 UTC)
        let (Some(slot), Some(locked), Some(datetime)) =
            (record.get(0), record.get(1), record.get(2))
        else {
            continue;
        };
        if let (Ok(slot), Ok(locked)) = (slot.parse::<u32>(), locked.parse::<u64>()) {
            rows.push(ScheduleRow {
                slot,
                locked,
                datetime: datetime.to_string(),
            });
        }
    }

    Rollups {
        // label lengths index into the ISO datetime "YYYY-MM-DDT..."
        all: rollup(&rows, 0),
        day: rollup(&rows, 10),
        month: rollup(&rows, 7),
        year: rollup(&rows, 4),
    }
});

/// Fold slot-ordered `rows` into buckets. `label_len` is the datetime prefix
/// length that keys a bucket (`0` = one all-time bucket, `4` = year, `7` =
/// month, `10` = day). The locked supply is non-increasing, so each bucket
/// reports its closing locked supply and the amount unlocked since the previous
/// bucket closed.
fn rollup(rows: &[ScheduleRow], label_len: usize) -> Vec<RollupRow> {
    let mut out = Vec::new();
    let Some(first) = rows.first() else {
        return out;
    };

    // Locked supply carried in from before the current bucket. Seeded with the
    // schedule's opening value so the first bucket's `unlocked` counts anything
    // vested within it.
    let mut prev_locked = first.locked;
    let mut label: Option<String> = None;
    let mut slot_end = 0;
    let mut locked_end = 0;

    for row in rows {
        let key = if label_len == 0 {
            "all".to_string()
        } else {
            // datetime is ASCII, so byte slicing is char-safe.
            row.datetime
                .get(..label_len)
                .unwrap_or(&row.datetime)
                .to_string()
        };

        if label.as_ref() == Some(&key) {
            slot_end = row.slot;
            locked_end = row.locked;
        } else {
            if let Some(date) = label.take() {
                out.push(RollupRow {
                    date,
                    global_slot: slot_end,
                    locked_supply: locked_end,
                    unlocked: prev_locked.saturating_sub(locked_end),
                });
                prev_locked = locked_end;
            }
            label = Some(key);
            slot_end = row.slot;
            locked_end = row.locked;
        }
    }

    if let Some(date) = label {
        out.push(RollupRow {
            date,
            global_slot: slot_end,
            locked_supply: locked_end,
            unlocked: prev_locked.saturating_sub(locked_end),
        });
    }

    out
}

#[derive(Default)]
pub struct TimeLocksQueryRoot;

#[Object]
impl TimeLocksQueryRoot {
    /// Locked/vesting supply rolled up by `bucket` (all-time / year / month /
    /// day), oldest first. Backs Blockberry's
    /// `getTimeLocks{All,Day,Month,Year}`.
    #[graphql(cache_control(max_age = 3600))]
    async fn time_locks(&self, bucket: TimeLockBucket) -> Result<Vec<TimeLockEntry>> {
        let rows = match bucket {
            TimeLockBucket::All => &ROLLUPS.all,
            TimeLockBucket::Day => &ROLLUPS.day,
            TimeLockBucket::Month => &ROLLUPS.month,
            TimeLockBucket::Year => &ROLLUPS.year,
        };
        Ok(rows.iter().map(TimeLockEntry::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollup_buckets_and_unlocked_deltas() {
        // A tiny monotonically-decreasing schedule spanning two days across a
        // year boundary: 100 -> 90 -> 70 (day 1), 70 -> 40 (day 2, next year).
        let rows = vec![
            ScheduleRow {
                slot: 0,
                locked: 100,
                datetime: "2021-12-31T00:00:00.000Z".into(),
            },
            ScheduleRow {
                slot: 1,
                locked: 90,
                datetime: "2021-12-31T00:03:00.000Z".into(),
            },
            ScheduleRow {
                slot: 2,
                locked: 70,
                datetime: "2021-12-31T23:57:00.000Z".into(),
            },
            ScheduleRow {
                slot: 3,
                locked: 40,
                datetime: "2022-01-01T00:00:00.000Z".into(),
            },
        ];

        // all-time: one bucket, closes at the final value, unlocked = 100 - 40.
        let all = rollup(&rows, 0);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].date, "all");
        assert_eq!(all[0].global_slot, 3);
        assert_eq!(all[0].locked_supply, 40);
        assert_eq!(all[0].unlocked, 60);

        // day: two buckets. Day 1 closes at 70 (unlocked 100-70=30); day 2
        // closes at 40 (unlocked 70-40=30, carried from the previous close).
        let day = rollup(&rows, 10);
        assert_eq!(day.len(), 2);
        assert_eq!(
            (day[0].date.as_str(), day[0].locked_supply, day[0].unlocked),
            ("2021-12-31", 70, 30)
        );
        assert_eq!(
            (day[1].date.as_str(), day[1].locked_supply, day[1].unlocked),
            ("2022-01-01", 40, 30)
        );

        // year: two buckets, same closes as the days here.
        let year = rollup(&rows, 4);
        assert_eq!(year.len(), 2);
        assert_eq!(
            (
                year[0].date.as_str(),
                year[0].locked_supply,
                year[0].unlocked
            ),
            ("2021", 70, 30)
        );
        assert_eq!(
            (
                year[1].date.as_str(),
                year[1].locked_supply,
                year[1].unlocked
            ),
            ("2022", 40, 30)
        );

        // unlocked telescopes to the total drop regardless of bucketing.
        assert_eq!(day.iter().map(|r| r.unlocked).sum::<u64>(), 60);
        assert_eq!(year.iter().map(|r| r.unlocked).sum::<u64>(), 60);
    }

    // Drives the resolver against the real embedded schedule, exercising the
    // `LazyLock` parse + GraphQL wiring end to end. Asserts the invariants that
    // hold for any monotonic schedule rather than mainnet-specific magic numbers,
    // so it doesn't break if `locked.csv` is refreshed.
    #[tokio::test]
    async fn time_locks_query_over_embedded_schedule() {
        use crate::{store::IndexerStore, web::graphql::build_schema};
        use std::sync::Arc;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexerStore::new(dir.path(), true).unwrap());
        let schema = build_schema(store, 0, 0, 0, false);

        let run = |bucket: &'static str| {
            let schema = schema.clone();
            async move {
                let q = format!(
                    "{{ timeLocks(bucket: {bucket}) \
                     {{ date global_slot locked_supply unlocked }} }}"
                );
                let res = schema.execute(q).await;
                assert!(res.errors.is_empty(), "timeLocks errored: {:?}", res.errors);
                res.data.into_json().unwrap()["timeLocks"]
                    .as_array()
                    .unwrap()
                    .clone()
            }
        };

        // all-time collapses to exactly one bucket.
        let all = run("ALL").await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["date"], "all");

        // finer granularity => at least as many buckets; slots strictly increase
        // (buckets are ordered) and locked supply is non-increasing (it only
        // vests). The all-time `unlocked` equals the sum of any granularity's.
        let year = run("YEAR").await;
        let month = run("MONTH").await;
        assert!(year.len() >= 1 && month.len() >= year.len());

        let total_unlocked = all[0]["unlocked"].as_u64().unwrap();
        let mut prev_slot = None;
        let mut prev_locked: Option<u64> = None;
        let mut summed = 0u64;
        for row in &year {
            let slot = row["global_slot"].as_u64().unwrap();
            let locked = row["locked_supply"].as_u64().unwrap();
            if let Some(p) = prev_slot {
                assert!(slot > p, "bucket slots must increase");
            }
            if let Some(p) = prev_locked {
                assert!(locked <= p, "locked supply must be non-increasing");
            }
            prev_slot = Some(slot);
            prev_locked = Some(locked);
            summed += row["unlocked"].as_u64().unwrap();
        }
        assert_eq!(
            summed, total_unlocked,
            "per-year unlocked must telescope to all-time"
        );
    }
}
