fn steps(start: i64) -> i64 {
    let mut n = start;
    let mut count = 0;
    while n != 1 {
        if n % 2 == 0 {
            n /= 2;
        } else {
            n = 3 * n + 1;
        }
        count += 1;
    }
    count
}

fn main() {
    let mut best = 0;
    let mut best_start = 0;
    for start in 1..std::hint::black_box(5_000_000i64) {
        let s = steps(start);
        if s > best {
            best = s;
            best_start = start;
        }
    }
    println!("{best_start} takes {best} steps");
}
