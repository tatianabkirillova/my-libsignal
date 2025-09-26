//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use criterion::{Criterion, criterion_group, criterion_main};
use libsignal_core::curve::KeyPair;
use rand::{Rng, rng};

pub fn generation(c: &mut Criterion) {
    let rng = &mut rng();
    c.bench_function("generation", |b| b.iter(|| KeyPair::generate(rng)));
}

pub fn key_agreement(c: &mut Criterion) {
    let rng = &mut rng();
    let alice_key = KeyPair::generate(rng);
    let bob_key = KeyPair::generate(rng);

    c.bench_function("key agreement", |b| {
        b.iter(|| alice_key.calculate_agreement(&bob_key.public_key).unwrap())
    });
}

pub fn signatures(c: &mut Criterion) {
    let rng = &mut rng();
    let alice_key = KeyPair::generate(rng);
    let mut some_data = [0; 1024];
    rng.fill(&mut some_data);

    c.bench_function("generate signature", |b| {
        b.iter(|| alice_key.calculate_signature(&some_data, rng).unwrap())
    });

    let sig = alice_key.calculate_signature(&some_data, rng).unwrap();

    c.bench_function("verify signature", |b| {
        b.iter(|| alice_key.public_key.verify_signature(&some_data, &sig))
    });
}

criterion_group!(benches, generation, key_agreement, signatures);

criterion_main!(benches);
