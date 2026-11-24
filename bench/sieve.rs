fn count_primes(limit: usize) -> usize {
    let mut composite = vec![false; limit + 1];
    let mut count = 0;
    for n in 2..=limit {
        if composite[n] {
            continue;
        }
        count += 1;
        let mut m = n * n;
        while m <= limit {
            composite[m] = true;
            m += n;
        }
    }
    count
}

fn main() {
    println!("{}", count_primes(std::hint::black_box(100_000_000)));
}
