fn main() {
    let mut seed: i64 = 7;
    let mut xs: Vec<i64> = Vec::new();
    for _ in 0..std::hint::black_box(3_000_000) {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        xs.push(seed);
    }
    xs.sort();
    let mut check = 0;
    for i in 0..xs.len() {
        if i % 1000 == 0 {
            check += xs[i] % 1000;
        }
    }
    println!("{check}");
}
