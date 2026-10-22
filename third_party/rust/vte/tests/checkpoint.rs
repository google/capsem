use std::sync::atomic::{AtomicUsize, Ordering};

use vte::ansi::{Handler, Processor};

#[derive(Default)]
struct Recorder {
    text: String,
    titles: Vec<Option<String>>,
    bells: usize,
}

impl Handler for Recorder {
    fn input(&mut self, c: char) {
        self.text.push(c);
    }

    fn set_title(&mut self, title: Option<String>) {
        self.titles.push(title);
    }

    fn bell(&mut self) {
        self.bells += 1;
    }
}

#[test]
fn continuation_matches_at_every_byte_split_without_replaying_effects() {
    let input = "before\x07\x1b]2;title\x07\x1b[38:2::1:2:3m界e\u{301}🙂\x1b[3bafter".as_bytes();
    for split in 0..=input.len() {
        let mut original = Processor::new();
        let mut first = Recorder::default();
        original.advance(&mut first, &input[..split]);
        let checkpoint = original.checkpoint(8192).unwrap();
        let encoded = serde_json::to_vec(&checkpoint).unwrap();
        let checkpoint = serde_json::from_slice(&encoded).unwrap();
        let mut restored = Processor::from_checkpoint(checkpoint, 8192).unwrap();
        let mut expected = Recorder::default();
        let mut actual = Recorder::default();
        original.advance(&mut expected, &input[split..]);
        restored.advance(&mut actual, &input[split..]);
        assert_eq!(actual.text, expected.text, "split {split}");
        assert_eq!(actual.titles, expected.titles, "split {split}");
        assert_eq!(actual.bells, expected.bells, "split {split}");
    }
}

#[test]
fn synchronized_output_is_restored_unprocessed_and_terminates_once() {
    let mut original = Processor::new();
    let mut first = Recorder::default();
    original.advance(&mut first, b"before\x1b[?2026hheld\x07\x1b]2;held-title\x07");
    assert_eq!(first.text, "before");
    assert_eq!(first.bells, 0);
    let checkpoint = original.checkpoint(8192).unwrap();
    let mut restored = Processor::from_checkpoint(checkpoint, 8192).unwrap();
    assert_eq!(restored.sync_bytes_count(), original.sync_bytes_count());
    let mut actual = Recorder::default();
    restored.advance(&mut actual, b"\x1b[?2026lafter");
    assert_eq!(actual.text, "heldafter");
    assert_eq!(actual.bells, 1);
    assert_eq!(actual.titles, [Some("held-title".into())]);
    assert_eq!(restored.sync_bytes_count(), 0);
}

#[test]
fn pending_buffers_are_bounded_before_copying_or_restoring() {
    let mut parser = Processor::new();
    parser.advance(&mut Recorder::default(), b"\x1b]2;long-title");
    assert!(parser.checkpoint(4).is_err());
    let checkpoint = parser.checkpoint(8192).unwrap();
    assert!(Processor::from_checkpoint(checkpoint, 4).is_err());

    let mut parser = Processor::new();
    parser.advance(&mut Recorder::default(), b"\x1b[?2026hbuffered");
    assert!(parser.checkpoint(4).is_err());
    let checkpoint = parser.checkpoint(8192).unwrap();
    assert!(Processor::from_checkpoint(checkpoint, 4).is_err());
}

#[test]
fn corrupt_parser_indices_and_parameter_groups_are_refused() {
    let mut parser = Processor::new();
    parser.advance(&mut Recorder::default(), b"\x1b[1;38:2::1");
    let original = serde_json::to_value(parser.checkpoint(8192).unwrap()).unwrap();
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("intermediate_idx", serde_json::json!(3)),
        ("osc_num_params", serde_json::json!(17)),
        ("partial_utf8_len", serde_json::json!(4)),
    ] {
        let mut corrupt = original.clone();
        corrupt["parser"][key] = value;
        let checkpoint = serde_json::from_value(corrupt).unwrap();
        assert!(Processor::from_checkpoint(checkpoint, 8192).is_err(), "{key}");
    }
    for (key, value) in [("len", 33), ("current_subparams", 32)] {
        let mut corrupt = original.clone();
        corrupt["parser"]["params"][key] = value.into();
        let checkpoint = serde_json::from_value(corrupt).unwrap();
        assert!(Processor::from_checkpoint(checkpoint, 8192).is_err(), "{key}");
    }
    let mut corrupt = original.clone();
    corrupt["parser"]["params"]["subparams"][0] = 0.into();
    let checkpoint = serde_json::from_value(corrupt).unwrap();
    assert!(Processor::from_checkpoint(checkpoint, 8192).is_err());
}

#[test]
fn parser_restore_never_dispatches_to_a_performer() {
    struct Counter;
    static ACTIONS: AtomicUsize = AtomicUsize::new(0);
    impl vte::Perform for Counter {
        fn print(&mut self, _: char) {
            ACTIONS.fetch_add(1, Ordering::SeqCst);
        }
    }
    let mut parser = vte::Parser::new();
    parser.advance(&mut Counter, b"x\x1b[31");
    let before = ACTIONS.load(Ordering::SeqCst);
    let checkpoint = parser.checkpoint(8192).unwrap();
    let mut restored = vte::Parser::from_checkpoint(checkpoint, 8192).unwrap();
    assert_eq!(ACTIONS.load(Ordering::SeqCst), before);
    restored.advance(&mut Counter, b"my");
    assert_eq!(ACTIONS.load(Ordering::SeqCst), before + 1);
}

#[test]
fn corrupt_osc_offsets_utf8_and_sync_deadlines_are_refused() {
    let mut parser = Processor::new();
    parser.advance(&mut Recorder::default(), b"\x1b]2;title");
    let original = serde_json::to_value(parser.checkpoint(8192).unwrap()).unwrap();
    for value in [serde_json::json!([0, 1000]), serde_json::json!([2, 1])] {
        let mut corrupt = original.clone();
        corrupt["parser"]["osc_params"][0] = value;
        let checkpoint = serde_json::from_value(corrupt).unwrap();
        assert!(Processor::from_checkpoint(checkpoint, 8192).is_err());
    }
    let mut corrupt = original.clone();
    corrupt["parser"]["partial_utf8_len"] = 1.into();
    corrupt["parser"]["partial_utf8"][0] = u32::from(b'a').into();
    let checkpoint = serde_json::from_value(corrupt).unwrap();
    assert!(Processor::from_checkpoint(checkpoint, 8192).is_err());
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("sync_remaining_nanos", serde_json::json!(151_000_000)),
        ("sync_buffer", serde_json::json!([65])),
    ] {
        let mut corrupt = original.clone();
        corrupt[key] = value;
        let checkpoint = serde_json::from_value(corrupt).unwrap();
        assert!(Processor::from_checkpoint(checkpoint, 8192).is_err(), "{key}");
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Actions(Vec<String>);

impl vte::Perform for Actions {
    fn print(&mut self, c: char) {
        self.0.push(format!("print {c:?}"));
    }
    fn execute(&mut self, byte: u8) {
        self.0.push(format!("execute {byte}"));
    }
    fn hook(&mut self, p: &vte::Params, i: &[u8], ignore: bool, c: char) {
        self.0.push(format!("hook {p:?} {i:?} {ignore} {c:?}"));
    }
    fn put(&mut self, byte: u8) {
        self.0.push(format!("put {byte}"));
    }
    fn unhook(&mut self) {
        self.0.push("unhook".into());
    }
    fn osc_dispatch(&mut self, p: &[&[u8]], bell: bool) {
        self.0.push(format!("osc {p:?} {bell}"));
    }
    fn csi_dispatch(&mut self, p: &vte::Params, i: &[u8], ignore: bool, c: char) {
        self.0.push(format!("csi {p:?} {i:?} {ignore} {c:?}"));
    }
    fn esc_dispatch(&mut self, i: &[u8], ignore: bool, byte: u8) {
        self.0.push(format!("esc {i:?} {ignore} {byte}"));
    }
}

#[test]
fn checkpoints_accept_reachable_hostile_states_and_preserve_all_actions() {
    let mut seed = 0x9137_6a21_u32;
    let mut byte = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 24) as u8
    };
    for round in 0..4096 {
        let prefix: Vec<_> = (0..round % 64).map(|_| byte()).collect();
        let suffix: Vec<_> = (0..64).map(|_| byte()).collect();
        let mut parser = vte::Parser::new();
        parser.advance(&mut Actions::default(), &prefix);
        let state = parser.checkpoint(8192).unwrap();
        let mut restored = vte::Parser::from_checkpoint(state, 8192).unwrap();
        let mut expected = Actions::default();
        let mut actual = Actions::default();
        parser.advance(&mut expected, &suffix);
        restored.advance(&mut actual, &suffix);
        assert_eq!(actual, expected, "round {round}, prefix {prefix:?}");
    }
}
