use super::*;

#[test]
fn is_main_thread_returns_false_on_worker() {
    let result = std::thread::spawn(is_main_thread).join().unwrap();
    assert!(!result);
}

#[test]
fn is_main_thread_returns_false_in_test_harness() {
    assert!(!is_main_thread());
}

#[tokio::test]
async fn is_main_thread_returns_false_in_tokio() {
    assert!(!is_main_thread());
}
