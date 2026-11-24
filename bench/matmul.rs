fn matrix(n: usize, seed: f64) -> Vec<Vec<f64>> {
    let mut m = Vec::new();
    for i in 0..n {
        let mut row = Vec::new();
        for j in 0..n {
            row.push(seed * (i as f64 - j as f64) * (i + j) as f64 / n as f64);
        }
        m.push(row);
    }
    m
}

fn multiply(a: &[Vec<f64>], b: &[Vec<f64>], n: usize) -> Vec<Vec<f64>> {
    let mut c = Vec::new();
    for i in 0..n {
        let mut row = vec![0.0; n];
        for k in 0..n {
            let aik = a[i][k];
            for j in 0..n {
                row[j] += aik * b[k][j];
            }
        }
        c.push(row);
    }
    c
}

fn main() {
    let n = std::hint::black_box(600);
    let a = matrix(n, 1.0);
    let b = matrix(n, 2.0);
    let c = multiply(&a, &b, n);
    let mut trace = 0.0;
    for i in 0..n {
        trace += c[i][i];
    }
    println!("{trace:.3}");
    println!("{:.3}", c[n / 2][n / 3]);
}
