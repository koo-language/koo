struct Entry {
    key: i64,
    id: i64,
}

fn main() {
    let mut seed: i64 = 7;
    let mut entries: Vec<Entry> = Vec::new();
    for i in 0..std::hint::black_box(1_500_000) {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        entries.push(Entry { key: seed % 100_000, id: i });
    }
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    let mut check = 0;
    for i in 0..entries.len() {
        if i % 1000 == 0 {
            check += entries[i].key % 1000 + entries[i].id % 7;
        }
    }
    println!("{check}");
}
