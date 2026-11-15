//! Replay a saved corpus and report completed versus rejected outcomes.
use std::{collections::BTreeMap, path::Path};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(
        args.len(),
        2,
        "usage: replay_corpus <machine_profile|machine_lowering> <directory>"
    );
    let run = match args[0].as_str() {
        "machine_profile" => alumina_interface_fuzz::machine_profile,
        "machine_lowering" => alumina_interface_fuzz::machine_lowering,
        _ => panic!("unknown target"),
    };
    let mut paths: Vec<_> = std::fs::read_dir(Path::new(&args[1]))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    let mut counts = BTreeMap::new();
    for path in paths {
        let bytes = std::fs::read(path).unwrap();
        let outcome = run(&bytes);
        *counts.entry(format!("{outcome:?}")).or_insert(0_usize) += 1;
    }
    for (name, count) in counts {
        println!("{name},{count}");
    }
}
