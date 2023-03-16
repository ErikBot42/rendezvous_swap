use criterion::{criterion_group, criterion_main, Criterion};
use rendezvous_swap::{Rendezvous, RendezvousData};
use std::thread;
use std::time::Instant;

fn bench(c: &mut Criterion) {
    c.bench_function("rendezvous swap and modify", move |b| {
        b.iter_custom(|iterations| {
            #[inline(always)]
            fn swap_increment(mut rendezvous: RendezvousData<i32>, iterations: u64) {
                for _ in 0..iterations {
                    *rendezvous.swap() += 1;
                }
            }
            let (rendezvous_0, rendezvous_1) = RendezvousData::new(0, 0);

            let handle = thread::spawn(move || {
                swap_increment(rendezvous_0, iterations);
            });

            let start = Instant::now();
            swap_increment(rendezvous_1, iterations);
            let time = start.elapsed();
            handle.join().unwrap();
            time
        })
    });

    c.bench_function("rendezvous", move |b| {
        b.iter_custom(|iterations| {
            #[inline(always)]
            fn wait(mut rendezvous: Rendezvous, iterations: u64) {
                for _ in 0..iterations {
                    rendezvous.wait();
                }
            }
            let (rendezvous_0, rendezvous_1) = Rendezvous::new();

            let handle = thread::spawn(move || {
                wait(rendezvous_0, iterations);
            });

            let start = Instant::now();
            wait(rendezvous_1, iterations);
            let time = start.elapsed();
            handle.join().unwrap();
            time
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
