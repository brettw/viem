//! Emit a complete portable preset from compiled defaults.
fn main() {
    let preset = match std::env::args().nth(1).as_deref() {
        None | Some("Midnight") => 0,
        Some("Paper") => 1,
        _ => panic!("usage: theme_defaults [Midnight|Paper]"),
    };
    use std::io::Write;
    std::io::stdout()
        .write_all(&viem_core::document::theme::default_json(preset).unwrap())
        .unwrap();
    println!();
}
