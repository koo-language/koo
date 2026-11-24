use std::collections::HashMap;

fn main() {
    let mut seed: i64 = 42;
    let mut counts: HashMap<String, i64> = HashMap::new();
    for _ in 0..std::hint::black_box(2_000_000) {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        let word = format!("w{}", seed % 5000);
        *counts.entry(word).or_insert(0) += 1;
    }
    let mut best = 0;
    for n in counts.values() {
        if *n > best {
            best = *n;
        }
    }
    println!("{} words, most frequent {best}", counts.len());
}
