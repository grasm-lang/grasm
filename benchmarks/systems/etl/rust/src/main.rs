//! Streaming JSON-to-database ETL.
//!
//! Reads a JSON Lines file line by line, deserializes each line into typed
//! structs with serde, validates, transforms, and inserts batches of 1000 into
//! an in-memory "database" that keeps one aggregate per city and a running
//! checksum. Each batch is dropped after its insert.
//!
//!     etl <path.jsonl>

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::process::ExitCode;

const BATCH: usize = 1000;
const MODULUS: i64 = 1_000_000_007;

#[derive(Deserialize)]
struct Address {
    city: String,
    #[allow(dead_code)]
    zip: String,
}

#[derive(Deserialize)]
struct User {
    name: String,
    email: String,
    tags: Vec<String>,
    address: Address,
}

#[derive(Deserialize)]
struct Order {
    #[allow(dead_code)]
    sku: String,
    qty: i64,
    price: i64,
}

#[derive(Deserialize)]
struct Customer {
    id: i64,
    user: User,
    orders: Vec<Order>,
    #[allow(dead_code)]
    note: String,
}

/// What the pipeline loads: one row per valid customer.
struct Row {
    id: i64,
    name: String,
    city: String,
    order_count: i64,
    total_cents: i64,
    tag_count: i64,
}

#[derive(Default)]
struct CityAgg {
    count: i64,
    total: i64,
}

#[derive(Default)]
struct Db {
    cities: BTreeMap<String, CityAgg>,
    rows: i64,
    total_cents: i64,
    checksum: i64,
}

impl Db {
    fn insert(&mut self, batch: Vec<Row>) {
        for r in batch {
            self.rows += 1;
            self.total_cents += r.total_cents;
            let mix = r.id * 7919
                + r.total_cents
                + r.order_count * 31
                + r.tag_count * 17
                + r.name.len() as i64;
            self.checksum = (self.checksum * 1_000_003 + mix) % MODULUS;
            let agg = self.cities.entry(r.city).or_default();
            agg.count += 1;
            agg.total += r.total_cents;
        }
    }

    fn city_digest(&self) -> i64 {
        self.cities.iter().fold(0, |d, (city, agg)| {
            (d * 131 + agg.count * 7 + agg.total + city.len() as i64) % MODULUS
        })
    }
}

fn validate(c: &Customer) -> bool {
    c.user.email.contains('@') && c.orders.iter().all(|o| o.qty > 0 && o.price >= 0)
}

fn transform(c: Customer) -> Row {
    Row {
        id: c.id,
        order_count: c.orders.len() as i64,
        total_cents: c.orders.iter().map(|o| o.qty * o.price).sum(),
        tag_count: c.user.tags.len() as i64,
        name: c.user.name,
        city: c.user.address.city,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        eprintln!("usage: etl <path.jsonl>");
        return ExitCode::from(2);
    }
    let file = match File::open(&args[0]) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("etl: cannot open {}: {e}", args[0]);
            return ExitCode::from(1);
        }
    };
    let mut reader = BufReader::with_capacity(1 << 20, file);

    let mut db = Db::default();
    let mut batch: Vec<Row> = Vec::with_capacity(BATCH);
    let mut rejected: i64 = 0;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("etl: read error: {e}");
                return ExitCode::from(1);
            }
        }
        let text = line.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            continue;
        }
        match serde_json::from_str::<Customer>(text) {
            Ok(c) if validate(&c) => {
                batch.push(transform(c));
                if batch.len() == BATCH {
                    db.insert(std::mem::replace(&mut batch, Vec::with_capacity(BATCH)));
                }
            }
            _ => rejected += 1,
        }
    }
    if !batch.is_empty() {
        db.insert(batch);
    }

    println!(
        "valid={} rejected={} cities={} total_cents={} checksum={} city_digest={}",
        db.rows,
        rejected,
        db.cities.len(),
        db.total_cents,
        db.checksum,
        db.city_digest()
    );
    ExitCode::SUCCESS
}
