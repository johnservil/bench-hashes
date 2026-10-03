// Pieces that arrive faster than one thread can hash them; each piece is
// borrowed only until the call returns.
use std::io::Read;

fn main() -> std::io::Result<()> {
    blake3_servil::initialize_multithreaded();
    let mut file = std::fs::File::open("Cargo.toml")?;
    let mut buffer = vec![0u8; 1 << 20];
    let mut hasher = blake3_servil::Hasher::new();
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        // Helper threads share each piece of 512 KiB or more; shorter
        // pieces hash on this thread.
        hasher.update_multithreaded(&buffer[..n]);
    }
    println!("{}", hasher.finalize().to_hex());
    Ok(())
}
