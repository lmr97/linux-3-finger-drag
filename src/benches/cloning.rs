// This doesn't exactly conform to a proper Rust
// benchmark, but it does show that that the cloning
// operations are extremely low-cost, on the order of
// less than a microsecond for both (closer to 200 ns 
// max for the configuration to be cloned).

use std::hint::black_box;
use criterion::{criterion_group, criterion_main, Criterion};
use linux_3_finger_drag::{
    init::config::Configuration,
    runtime::virtual_trackpad
};

pub fn clone_virtual_trackpad(c: &mut Criterion) {
    let vtp = black_box(virtual_trackpad::start_handler().unwrap());
    
    c.bench_function("vtrack cloning", |b| {
        b.iter(|| {
            let vtp2 = vtp.clone();
            black_box(vtp2);
        });
    });
}

fn clone_config(c: &mut Criterion) {
    let cfg = Configuration::default();
    
    c.bench_function("config cloning", |b| {
        b.iter(|| {
            let cfg2 = cfg.clone();
            std::hint::black_box(cfg2);
        });
    });
}

criterion_group!(clones, clone_config, clone_virtual_trackpad);
criterion_main!(clones);