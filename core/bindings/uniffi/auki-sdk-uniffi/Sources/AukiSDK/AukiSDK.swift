/// Marker for the hand-written Swift module that hosts generated UniFFI glue.
/// Platform lifecycle conveniences will remain small extensions in this
/// target; authentication and networking stay in Rust.
public enum AukiSDKModule {
    public static let version = "0.1.0"
}
