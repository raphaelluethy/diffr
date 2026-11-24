//! Prints a file's highlight segments: `cargo run --example dump -- <file>`.
fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <file>");
    let text = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let Some(segs) = diffr_highlight::lines(&path, &lines) else {
        println!("no highlighting");
        return;
    };
    for (line, segs) in lines.iter().zip(&segs) {
        println!("{line}");
        let parts: Vec<String> = segs
            .iter()
            .map(|s| {
                format!(
                    "{:?}:{:?}",
                    s.class,
                    &line[s.start as usize..s.end as usize]
                )
            })
            .collect();
        println!("    {}", parts.join(" "));
    }
}
