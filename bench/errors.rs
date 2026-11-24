enum Error {
    Negative { value: i64 },
    TooLarge,
}

fn checked(n: i64) -> Result<i64, Error> {
    if n < 0 {
        return Err(Error::Negative { value: n });
    }
    if n > 1_000_000 {
        return Err(Error::TooLarge);
    }
    Ok(n * 2)
}

fn pipeline(n: i64) -> Result<i64, Error> {
    let a = checked(n)?;
    let b = checked(a / 2 + 1)?;
    checked(b / 2 - 3)
}

fn main() {
    let (mut ok, mut failed, mut sum) = (0i64, 0i64, 0i64);
    for i in 0..std::hint::black_box(60_000_000i64) {
        match pipeline(i % 1_000_100 - 50) {
            Ok(v) => {
                ok += 1;
                sum += v;
            }
            Err(Error::Negative { value }) => {
                failed += 1;
                sum -= value;
            }
            Err(Error::TooLarge) => failed += 1,
        }
    }
    println!("{ok} ok, {failed} failed, sum {sum}");
}
