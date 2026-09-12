use super::*;

#[tokio::test]
async fn frames_round_trip_in_order() {
    let mut wire = Vec::new();
    write_frame(&mut wire, &[1, 2, 3]).await.unwrap();
    write_frame(&mut wire, &[9; 1500]).await.unwrap();
    let mut reader = std::io::Cursor::new(wire);
    let mut packet = Vec::new();
    assert_eq!(read_frame(&mut reader, &mut packet).await.unwrap(), Some(3));
    assert_eq!(packet, [1, 2, 3]);
    assert_eq!(read_frame(&mut reader, &mut packet).await.unwrap(), Some(1500));
    assert_eq!(packet, [9; 1500]);
    assert_eq!(read_frame(&mut reader, &mut packet).await.unwrap(), None, "clean end");
}

#[tokio::test]
async fn a_frame_cut_short_is_an_error_not_an_end() {
    let mut wire = Vec::new();
    write_frame(&mut wire, &[7; 40]).await.unwrap();
    wire.truncate(20);
    let mut reader = std::io::Cursor::new(wire);
    let error = read_frame(&mut reader, &mut Vec::new()).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[tokio::test]
async fn an_empty_frame_is_refused_on_both_sides() {
    let mut reader = std::io::Cursor::new(vec![0, 0]);
    assert_eq!(
        read_frame(&mut reader, &mut Vec::new()).await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        write_frame(&mut Vec::new(), &[]).await.unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn the_largest_frame_fits_and_one_more_byte_does_not() {
    let mut wire = Vec::new();
    write_frame(&mut wire, &vec![1; MAX_FRAME_BYTES]).await.unwrap();
    let mut packet = Vec::new();
    assert_eq!(
        read_frame(&mut std::io::Cursor::new(wire), &mut packet).await.unwrap(),
        Some(MAX_FRAME_BYTES)
    );
    assert!(write_frame(&mut Vec::new(), &vec![1; MAX_FRAME_BYTES + 1])
        .await
        .is_err());
}
