#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: generate_identity <path>")?;
    if args.next().is_some() {
        return Err("usage: generate_identity <path>".into());
    }

    let identity = auki_sdk::Identity::load_or_create(path)?;
    println!("peer: {}", identity.peer_id());
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("Identity files require a native target.");
}
