//! How long does mom take to look at the machine? `cargo run --release -p mom-core --example bench`
use std::time::Instant;

fn main() {
    let mut sampler = mom_core::procs::Sampler::new();
    let t = Instant::now();
    let table = sampler.sample();
    println!("first look: {:?} for {} processes", t.elapsed(), table.procs.len());
    let t = Instant::now();
    let table = sampler.sample();
    println!("next look (details cached): {:?}", t.elapsed());
    let t = Instant::now();
    let listeners = mom_core::ports::listeners();
    println!("ports: {:?}", t.elapsed());
    let t = Instant::now();
    let snap = mom_core::build_snapshot(&table, &listeners, &mom_core::Config::default(), Default::default());
    println!("analysis: {:?} ({} groups, {} leftovers)", t.elapsed(), snap.groups.len(), snap.summary.leftovers);
}
