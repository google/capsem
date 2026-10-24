use super::*;
use std::cell::Cell;

struct Counted<'a> {
    calls: &'a Cell<usize>,
    bytes: &'a [u8],
}

impl Serialize for Counted<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.calls.set(self.calls.get() + 1);
        serializer.serialize_bytes(self.bytes)
    }
}

#[test]
fn final_frame_serializes_once_and_matches_original_wire_at_exact_bound() {
    let calls = Cell::new(0);
    let message = Counted {
        calls: &calls,
        bytes: b"original\0payload",
    };
    let payload = rmp_serde::to_vec_named(&message).unwrap();
    calls.set(0);
    let frame = encode_frame(&message, payload.len() as u32).unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(&frame[..4], &(payload.len() as u32).to_be_bytes());
    assert_eq!(&frame[4..], payload);
    calls.set(0);
    assert!(encode_frame(&message, payload.len() as u32 - 1).is_err());
    assert_eq!(calls.get(), 1);
}

#[test]
fn refused_append_cannot_write_past_budget_or_touch_reserved_header() {
    let calls = Cell::new(0);
    let message = Counted {
        calls: &calls,
        bytes: &[0; 128],
    };
    let mut frame = vec![7; 4];
    assert!(append_record(&mut frame, &message, 10).is_err());
    assert_eq!(calls.get(), 1);
    assert_eq!(&frame[..4], &[7; 4]);
    assert!(frame.len() <= 14);
}

#[test]
fn prepared_frame_replays_original_storage_without_another_serialization() {
    let calls = Cell::new(0);
    let message = Counted {
        calls: &calls,
        bytes: b"original prepared payload",
    };
    let frame = PreparedFrame::new(&message, 1024).unwrap();
    assert_eq!(calls.get(), 1);
    let replay = frame.clone();
    assert_eq!(frame.bytes().as_ptr(), replay.bytes().as_ptr());
    assert_eq!(frame.bytes(), replay.bytes());
    assert_eq!(calls.get(), 1);
}

#[test]
fn prepared_frame_does_not_retain_the_original_payload() {
    struct Payload<'a> {
        dropped: &'a Cell<bool>,
        data: Vec<u8>,
    }
    impl Drop for Payload<'_> {
        fn drop(&mut self) {
            self.dropped.set(true);
        }
    }
    impl Serialize for Payload<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(&self.data)
        }
    }
    let dropped = Cell::new(false);
    let message = Payload {
        dropped: &dropped,
        data: vec![19; 8192],
    };
    let frame = PreparedFrame::new(&message, 16384).unwrap();
    drop(message);
    assert!(dropped.get());
    let replay = frame.clone();
    assert_eq!(replay.bytes().as_ptr(), frame.bytes().as_ptr());
    assert_eq!(
        rmp_serde::from_slice::<serde_bytes::ByteBuf>(&replay.bytes()[4..])
            .unwrap()
            .as_ref(),
        &[19; 8192]
    );
}

#[test]
fn final_buffer_capacity_cannot_double_past_the_selected_byte_ceiling() {
    #[derive(Serialize)]
    struct Tail<'a> {
        #[serde(with = "serde_bytes")]
        data: &'a [u8],
        tail: u8,
    }
    let message = Tail {
        data: &[7; 8192],
        tail: 1,
    };
    let payload = rmp_serde::to_vec_named(&message).unwrap();
    let frame = encode_frame(&message, payload.len() as u32).unwrap();
    assert!(
        frame.capacity() <= payload.len() + 4,
        "{} > {}",
        frame.capacity(),
        payload.len() + 4
    );
    let record = encode_record(&1_u8, 1).unwrap();
    assert_eq!(record.capacity(), 1);
}

#[test]
fn encoding_failure_is_not_misclassified_as_size_refusal() {
    struct Refused;
    impl Serialize for Refused {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("original serializer refusal"))
        }
    }
    let failed = encode_frame(&Refused, 1024).unwrap_err();
    assert!(!is_frame_too_large(&failed));
    let large = encode_frame(&[0_u8; 32], 1).unwrap_err();
    assert!(is_frame_too_large(&large));
}
