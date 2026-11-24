use std::fmt::Write;

struct Row {
    id: i64,
    name: String,
    score: f64,
}

fn render(row: &Row) -> String {
    format!("id={} name={} score={:.2}", row.id, row.name, row.score)
}

fn main() {
    let mut rows: Vec<Row> = Vec::new();
    for i in 0..std::hint::black_box(200_000i64) {
        rows.push(Row { id: i, name: format!("user{}", i % 1000), score: i as f64 / 7.0 });
    }
    let mut total = 0;
    let mut report = String::new();
    for _round in 0..10 {
        report = String::new();
        for row in &rows {
            let line = render(row);
            total += line.len();
            report.push_str(&line);
            report.push('\n');
        }
    }
    let _ = write!(report, "");
    println!("{total}");
    println!("{}", report.len());
}
