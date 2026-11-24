fn main() {
    let xs: Vec<i64> = (0..std::hint::black_box(3_000_000i64)).collect();
    let mut total = 0i64;
    for round in 0..10 {
        total += xs.iter().map(|x| x * 3 + round).filter(|x| x % 2 == 0).fold(0, |a, x| a + x % 7);
    }
    println!("{total}");
}
