//! Reusable retained-history memory benchmark, independent of the fuzz oracle.
//! Run one policy per process; see docs/fuzzing.md.
#[allow(dead_code)]
#[path = "fuzz_backend/support.rs"]
mod support;

use evim_core::document::{Document, Encoding, Format, HistoryRetentionPolicy};
use serde_json::json;

#[global_allocator]
static ALLOCATOR: support::TrackingAllocator = support::TrackingAllocator;

fn main() {
    let mut policy_name = "default".to_string();
    let mut seed = 20_260_966_u64;
    let mut steps = 1000_usize;
    let mut source_characters = 3000_usize;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--help" {
            println!("history_memory --policy default|unlimited|128 --seed N --steps N --source-characters N");
            return;
        }
        let value = args.next().expect("option requires a value");
        match argument.as_str() {
            "--policy" => policy_name = value,
            "--seed" => seed = value.parse().expect("invalid seed"),
            "--steps" => steps = value.parse().expect("invalid steps"),
            "--source-characters" => {
                source_characters = value.parse().expect("invalid source size")
            }
            _ => panic!("unknown option {argument}"),
        }
    }
    assert!(steps > 0 && source_characters > 0);
    let policy = match policy_name.as_str() {
        "default" => HistoryRetentionPolicy::default(),
        "unlimited" => HistoryRetentionPolicy::unlimited(),
        "128" => HistoryRetentionPolicy::new(128, usize::MAX),
        _ => panic!("unknown history policy"),
    };
    let baseline = support::memory();
    let mut expected: String = ['é', 'π', 'a']
        .into_iter()
        .cycle()
        .take(source_characters)
        .collect();
    let bytes: Vec<_> = expected.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut document = Document::from_bytes(bytes, Encoding::Utf16Le, Format::PlainText).unwrap();
    document.set_history_retention_policy(policy);
    let mut random = support::Rng::new(seed);
    println!(
        "{}",
        json!({"type":"header","policy":policy_name,"seed":seed,"steps":steps,
        "source_characters":source_characters,"baseline":baseline})
    );
    for index in 0..steps {
        let character_index = random.usize(source_characters);
        let (start, old) = expected.char_indices().nth(character_index).unwrap();
        let replacement = match old {
            'é' => "π",
            'π' => "a",
            _ => "é",
        };
        let end = start + old.len_utf8();
        expected.replace_range(start..end, replacement);
        document.replace(start..end, replacement).unwrap();
        assert_eq!(document.text(), expected);
        if (index + 1) % 100 == 0 || index + 1 == steps {
            let status = document.history_status();
            println!(
                "{}",
                json!({"type":"sample","steps":index+1,"nodes":status.node_count,
                "source_bytes":document.source_bytes().len(),"retained_source_bytes":status.retained_source_bytes,
                "retained_memory_bytes":status.retained_memory_bytes,"memory":support::memory()})
            );
        }
    }
    let saved = document.source_bytes();
    let expected_bytes: Vec<_> = expected.encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(saved, expected_bytes);
    if document.history_status().can_undo {
        document.try_undo().unwrap();
        document.try_redo().unwrap();
        assert_eq!(document.source_bytes(), saved);
    }
    document.set_history_retention_policy(HistoryRetentionPolicy::new(1, usize::MAX));
    println!(
        "{}",
        json!({"type":"pruned_to_one","retained_memory_bytes":document.history_status().retained_memory_bytes,
        "memory":support::memory()})
    );
    drop((document, expected, saved, expected_bytes));
    println!("{}", json!({"type":"dropped","memory":support::memory()}));
}
