use auki_uniffi_test::{
    Counter, GreetingStyle, TestError, add, delayed_greeting, hello, make_greeting,
};

#[test]
fn sync_exports_return_plain_values() {
    assert_eq!(add(2, 40), 42);
    assert_eq!(hello("Auki".to_string()), "Hello, Auki.");
}

#[test]
fn record_and_enum_cross_the_surface() {
    let greeting = make_greeting("Nils".to_string(), GreetingStyle::Formal).unwrap();

    assert_eq!(greeting.message, "Good day, Nils.");
    assert_eq!(greeting.name_length, 4);
    assert_eq!(greeting.style, GreetingStyle::Formal);
}

#[test]
fn empty_names_return_a_typed_error() {
    let err = make_greeting("".to_string(), GreetingStyle::Casual).unwrap_err();

    assert!(matches!(err, TestError::EmptyName));
}

#[tokio::test]
async fn async_export_returns_record_after_delay() {
    let greeting = delayed_greeting("Swift".to_string(), 0)
        .await
        .expect("valid async greeting");

    assert_eq!(greeting.message, "Hello, Swift.");
    assert_eq!(greeting.name_length, 5);
    assert_eq!(greeting.style, GreetingStyle::Casual);
}

#[tokio::test]
async fn async_export_rejects_large_delay() {
    let err = delayed_greeting("Swift".to_string(), 1_001)
        .await
        .unwrap_err();

    assert!(matches!(err, TestError::DelayTooLarge { max_ms: 1_000 }));
}

#[tokio::test]
async fn object_export_holds_state_across_async_calls() {
    let counter = Counter::new(10);

    assert_eq!(counter.value(), 10);
    assert_eq!(counter.add_after(5, 0).await.unwrap(), 15);
    assert_eq!(counter.value(), 15);
}
