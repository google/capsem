use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::checkpoint::{CheckpointLimits, TermCheckpoint};
use alacritty_terminal::term::{Config, Osc52, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use alacritty_terminal::Term;

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<Event>>>);
impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}
struct Size(usize, usize);
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.1
    }
    fn screen_lines(&self) -> usize {
        self.1
    }
    fn columns(&self) -> usize {
        self.0
    }
}
fn config() -> Config {
    Config {
        scrolling_history: 100,
        osc52: Osc52::Disabled,
        kitty_keyboard: true,
        ..Config::default()
    }
}
fn limits() -> CheckpointLimits {
    CheckpointLimits {
        max_columns: 256,
        max_lines: 100,
        max_history: 100,
        max_cells: 32768,
        max_text_bytes: 8192,
        max_combining_chars: 64,
    }
}
fn state(term: &Term<Events>) -> serde_json::Value {
    let mut state = serde_json::to_value(term.checkpoint(&limits()).unwrap()).unwrap();
    // This process-global high-water mark may advance in another test's Term.
    state["hyperlink_id_suffix"] = serde_json::Value::Null;
    state
}
fn restore(term: &Term<Events>) -> (Term<Events>, Events) {
    let bytes = rmp_serde::to_vec_named(&term.checkpoint(&limits()).unwrap()).unwrap();
    let checkpoint: TermCheckpoint = rmp_serde::from_slice(&bytes).unwrap();
    let events = Events::default();
    let restored = Term::from_checkpoint(checkpoint, config(), events.clone(), &limits()).unwrap();
    assert!(events.0.lock().unwrap().is_empty(), "restore emitted terminal effects");
    (restored, events)
}

#[test]
fn both_screens_cursors_modes_palette_and_stacks_continue_exactly() {
    let mut term = Term::new(config(), &Size(30, 8), Events::default());
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, b"PRIMARY\x1b[3;7H\x1b7\x1b]2;old-title\x07\x1b[22;0t\x1b]2;new-title\x07\x1b[?1h\x1b[?2004h\x1b[4;7r\x1b[?1049h\x1b[HALTERNATE\x1b[4;8H\x1b7\x1b[38;2;1;2;3m\x1b]4;7;rgb:12/34/56\x07\x1b[>3u");
    let (mut restored, events) = restore(&term);
    assert_eq!(state(&restored), state(&term));
    assert!(restored.mode().contains(TermMode::ALT_SCREEN));
    assert!(restored
        .mode()
        .contains(TermMode::APP_CURSOR | TermMode::BRACKETED_PASTE));
    assert_eq!(restored.grid().cursor.point, Point::new(Line(3), Column(7)));
    assert_eq!(restored.grid().saved_cursor.point, Point::new(Line(3), Column(7)));
    assert_eq!(
        restored.colors()[7],
        Some(alacritty_terminal::vte::ansi::Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56
        })
    );
    let suffix = b"X\x1b8Y\x1b[?1049l\x1b8Z\x1b[23;0t\x1b[<u";
    parser.advance(&mut term, suffix);
    let mut next: Processor = Processor::new();
    next.advance(&mut restored, suffix);
    assert_eq!(state(&restored), state(&term));
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|event| matches!(event, Event::Title(title) if title == "old-title")));
}

#[test]
fn wrap_charsets_combining_hyperlinks_and_reflow_survive() {
    let mut term = Term::new(config(), &Size(10, 3), Events::default());
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, "\x1b]8;id=owned;https://example.invalid/link\x1b\\界e\u{301}\x1b]8;;\x1b\\abcdefg\r\nnext\r\nthird\r\nhistory\x1b(0q\x1b7".as_bytes());
    let (mut restored, _) = restore(&term);
    for size in [Size(5, 4), Size(20, 6), Size(8, 2)] {
        term.resize(Size(size.0, size.1));
        restored.resize(size);
        assert_eq!(state(&restored), state(&term));
    }
    let suffix = b"x\x1b8y\x1b(Bz";
    parser.advance(&mut term, suffix);
    let mut next: Processor = Processor::new();
    next.advance(&mut restored, suffix);
    assert_eq!(state(&restored), state(&term));
}

#[test]
fn complete_engine_and_parser_resume_each_fragment_without_effect_replay() {
    let bytes = b"\x07\x1b]2;title\x07\x1b[38:2::1:2:3mtext\x1b[?1049h\x1b[3;4Halt\x1b[?1049l";
    for split in 0..=bytes.len() {
        let mut term = Term::new(config(), &Size(20, 6), Events::default());
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, &bytes[..split]);
        let processor = parser.checkpoint(8192).unwrap();
        let (mut restored, _) = restore(&term);
        let mut resumed = Processor::from_checkpoint(processor, 8192).unwrap();
        parser.advance(&mut term, &bytes[split..]);
        resumed.advance(&mut restored, &bytes[split..]);
        assert_eq!(state(&restored), state(&term), "split {split}");
    }
}

#[test]
fn malformed_grid_cursor_tabs_palette_and_policy_are_refused() {
    let term = Term::new(config(), &Size(20, 6), Events::default());
    let original = serde_json::to_value(term.checkpoint(&limits()).unwrap()).unwrap();
    let mutations = [
        ("/version", serde_json::json!(2)),
        ("/grid/columns", serde_json::json!(0)),
        ("/grid/lines", serde_json::json!(0)),
        ("/grid/display_offset", serde_json::json!(1000)),
        ("/grid/max_scroll_limit", serde_json::json!(101)),
        ("/grid/cursor/point/column", serde_json::json!(20)),
        ("/inactive_grid/saved_cursor/point/line", serde_json::json!(6)),
        ("/grid/rows/0/occ", serde_json::json!(21)),
        ("/tabs", serde_json::json!([])),
        ("/colors", serde_json::json!([])),
        ("/scroll_region/end", serde_json::json!(7)),
        ("/config/osc52", serde_json::json!("copypaste")),
    ];
    for (pointer, value) in mutations {
        let mut corrupt = original.clone();
        *corrupt.pointer_mut(pointer).unwrap() = value;
        let checkpoint = serde_json::from_value(corrupt).unwrap();
        assert!(
            Term::from_checkpoint(checkpoint, config(), Events::default(), &limits()).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn cell_and_text_budgets_apply_before_checkpoint_copy_and_restore() {
    let mut term = Term::new(config(), &Size(20, 6), Events::default());
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, b"\x1b]2;too-long-title\x07");
    let mut constrained = limits();
    constrained.max_text_bytes = 4;
    assert!(term.checkpoint(&constrained).is_err());
    constrained = limits();
    constrained.max_cells = 100;
    assert!(term.checkpoint(&constrained).is_err());
    let checkpoint = term.checkpoint(&limits()).unwrap();
    assert!(Term::from_checkpoint(checkpoint, config(), Events::default(), &constrained).is_err());
    term.grid_mut()[Point::new(Line(0), Column(0))].push_zerowidth('\u{301}');
    constrained = limits();
    constrained.max_combining_chars = 0;
    assert!(term.checkpoint(&constrained).is_err());
}

#[test]
fn custom_tabs_cursor_shape_focus_and_selection_survive() {
    use alacritty_terminal::index::Side;
    use alacritty_terminal::selection::{Selection, SelectionType};
    use alacritty_terminal::vi_mode::ViModeCursor;
    use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};
    let mut term = Term::new(config(), &Size(20, 6), Events::default());
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, b"selected\x1b[3g\x1b[1;6H\x1bH\x1b[6 q");
    term.is_focused = true;
    term.vi_mode_cursor = ViModeCursor::new(Point::new(Line(1), Column(3)));
    let mut selection = Selection::new(SelectionType::Simple, Point::new(Line(0), Column(0)), Side::Left);
    selection.update(Point::new(Line(0), Column(7)), Side::Right);
    term.selection = Some(selection);
    let (mut restored, _) = restore(&term);
    assert!(restored.is_focused);
    assert_eq!(restored.vi_mode_cursor.point, Point::new(Line(1), Column(3)));
    assert_eq!(restored.selection_to_string().as_deref(), Some("selected"));
    assert_eq!(
        restored.cursor_style(),
        CursorStyle {
            shape: CursorShape::Beam,
            blinking: false
        }
    );
    let mut next: Processor = Processor::new();
    next.advance(&mut restored, b"\x1b[1;1H\tZ");
    assert_eq!(restored.grid()[Point::new(Line(0), Column(5))].c, 'Z');
}

#[test]
fn owner_restore_retains_hyperlinks_and_advances_automatic_id_counter() {
    let mut term = Term::new(config(), &Size(20, 6), Events::default());
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut term,
        b"\x1b]8;id=retained;https://example.invalid/old\x07old\x1b]8;;\x07",
    );
    let mut value = serde_json::to_value(term.checkpoint(&limits()).unwrap()).unwrap();
    let next_id = value["hyperlink_id_suffix"].as_u64().unwrap() + 50;
    value["hyperlink_id_suffix"] = next_id.into();
    let checkpoint = serde_json::from_value(value).unwrap();
    let mut restored = Term::from_checkpoint(checkpoint, config(), Events::default(), &limits()).unwrap();
    let link = restored.grid()[Point::new(Line(0), Column(0))].hyperlink().unwrap();
    assert_eq!(link.id(), "retained");
    assert_eq!(link.uri(), "https://example.invalid/old");
    let mut next: Processor = Processor::new();
    next.advance(&mut restored, b"\x1b]8;;https://example.invalid/new\x07N\x1b]8;;\x07");
    let link = restored.grid()[Point::new(Line(0), Column(3))].hyperlink().unwrap();
    assert!(link.id().strip_suffix("_alacritty").unwrap().parse::<u64>().unwrap() >= next_id);
    assert_eq!(link.uri(), "https://example.invalid/new");
}

#[test]
fn all_upstream_recordings_continue_through_engine_and_parser_checkpoints() {
    use std::path::Path;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ref");
    let mut directories: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    directories.sort();
    assert!(!directories.is_empty());
    let budget = CheckpointLimits {
        max_columns: 256,
        max_lines: 128,
        max_history: 1500,
        max_cells: 1_000_000,
        max_text_bytes: 8192,
        max_combining_chars: 64,
    };
    let normalize = |term: &Term<Events>| {
        let mut value = serde_json::to_value(term.checkpoint(&budget).unwrap()).unwrap();
        value["hyperlink_id_suffix"] = serde_json::Value::Null;
        value
    };
    for directory in directories {
        let size: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("size.json")).unwrap()).unwrap();
        let settings: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("config.json")).unwrap()).unwrap();
        let input = std::fs::read(directory.join("alacritty.recording")).unwrap();
        let mut policy = config();
        policy.scrolling_history = settings["history_size"].as_u64().unwrap_or(0) as usize;
        let mut original = Term::new(
            policy.clone(),
            &Size(
                size["columns"].as_u64().unwrap() as usize,
                size["screen_lines"].as_u64().unwrap() as usize,
            ),
            Events::default(),
        );
        let mut parser: Processor = Processor::new();
        let split = input.len() / 2;
        parser.advance(&mut original, &input[..split]);
        let checkpoint = original
            .checkpoint(&budget)
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()));
        let bytes = rmp_serde::to_vec_named(&checkpoint).unwrap();
        let mut restored = Term::from_checkpoint(
            rmp_serde::from_slice(&bytes).unwrap(),
            policy,
            Events::default(),
            &budget,
        )
        .unwrap();
        let mut resumed = Processor::from_checkpoint(parser.checkpoint(8192).unwrap(), 8192).unwrap();
        parser.advance(&mut original, &input[split..]);
        resumed.advance(&mut restored, &input[split..]);
        assert_eq!(normalize(&restored), normalize(&original), "{}", directory.display());
    }
}

#[test]
fn stored_tabs_survive_but_escape_cells_are_refused() {
    let mut term = Term::new(config(), &Size(20, 6), Events::default());
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, b"a\tb");
    assert_eq!(term.grid()[Point::new(Line(0), Column(1))].c, '\t');
    let (restored, _) = restore(&term);
    assert_eq!(state(&restored), state(&term));
    term.grid_mut()[Point::new(Line(0), Column(1))].c = '\x1b';
    assert!(term.checkpoint(&limits()).is_err());
}

#[test]
fn semantic_effects_reach_the_owner_once_and_restore_without_replay() {
    let events = Events::default();
    let mut term = Term::new(config(), &Size(20, 6), events.clone());
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut term,
        concat!(
            "\x1b]0;both\x07\x1b]1;icon\x07\x1b]2;title\x07",
            "\x07\x1bg\x1b]9;ready 界\x07\x1b]777;notify;agent;done\x07",
            "\x1b]52;c;aGVsbG8=\x07\x1b]52;c;?\x07"
        )
        .as_bytes(),
    );
    let actual: Vec<_> = events
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|event| match event {
            Event::IconName(name) => format!("icon:{name:?}"),
            Event::Title(title) => format!("title:{title}"),
            Event::Bell => "bell".to_owned(),
            Event::VisualBell => "visual-bell".to_owned(),
            Event::Notification { title, body } => format!("notification:{title}:{body}"),
            other => panic!("unexpected owner event: {other:?}"),
        })
        .collect();
    assert_eq!(
        actual,
        [
            "icon:Some(\"both\")",
            "title:both",
            "icon:Some(\"icon\")",
            "title:title",
            "bell",
            "visual-bell",
            "notification::ready 界",
            "notification:agent:done",
        ]
    );
    let (mut restored, restored_events) = restore(&term);
    let mut resumed: Processor = Processor::new();
    resumed.advance(&mut restored, b"continuation");
    assert!(restored_events.0.lock().unwrap().is_empty());
    assert_eq!(
        format!(
            "{:?}",
            Event::Notification {
                title: "secret".into(),
                body: "private".into()
            }
        ),
        "Notification"
    );
}

#[test]
fn color_query_captures_the_override_before_later_palette_changes() {
    use alacritty_terminal::vte::ansi::Rgb;
    let events = Events::default();
    let mut term = Term::new(config(), &Size(20, 6), events.clone());
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut term,
        b"\x1b]4;1;rgb:12/34/56\x07\x1b]4;1;?\x07\x1b]4;1;rgb:aa/bb/cc\x07\x1b]4;2;?\x07",
    );
    let events = events.0.lock().unwrap();
    let replies: Vec<_> = events
        .iter()
        .map(|event| match event {
            Event::ColorRequest(_, format) => format(Rgb { r: 1, g: 2, b: 3 }),
            other => panic!("unexpected color event: {other:?}"),
        })
        .collect();
    assert_eq!(
        replies,
        ["\x1b]4;1;rgb:1212/3434/5656\x07", "\x1b]4;2;rgb:0101/0202/0303\x07"]
    );
}

#[test]
fn configured_combining_retention_is_bounded_during_live_input() {
    let mut policy = config();
    policy.max_combining_chars = Some(4);
    let mut term = Term::new(policy.clone(), &Size(20, 6), Events::default());
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, b"e");
    let _ = term.damage();
    term.reset_damage();
    for _ in 0..1000 {
        parser.advance(&mut term, "\u{301}".as_bytes());
    }
    let first = Point::new(Line(0), Column(0));
    assert_eq!(term.grid()[first].zerowidth().unwrap(), &['\u{301}'; 4]);
    let alacritty_terminal::term::TermDamage::Partial(damage) = term.damage() else {
        panic!("combining input must retain incremental damage");
    };
    assert!(damage.into_iter().any(|line| line.line == 0 && line.left == 0));
    parser.advance(&mut term, b"x");
    assert_eq!(term.grid()[Point::new(Line(0), Column(1))].c, 'x');
    let checkpoint = term.checkpoint(&limits()).unwrap();
    let restored = Term::from_checkpoint(checkpoint, policy, Events::default(), &limits()).unwrap();
    assert_eq!(restored.grid()[first].zerowidth().unwrap().len(), 4);
}
