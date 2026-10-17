// One long message read in pieces (a file, a socket): the stream reads it
// straight into its own memory, hashing each piece while it reads the next,
// and puts the digest in your place.
use blake3_servil::{Hash, Mode};

fn main() {
    // Once per process, before any other call.
    blake3_servil::initialize_multithreaded();
    let message = vec![7u8; 10 << 20]; // your reader goes here: a File, a TcpStream
    let mut digest = Hash::from_bytes([0; 32]);
    blake3_servil::stream(Mode::Hash, |_: &mut [Hash]| {}, |s| {
        let len = s.submit_reader(&message[..], Some(message.len() as u64), &mut digest).expect("the read");
        assert_eq!(len, message.len() as u64);
        s.finish();
    });
    println!("{}", digest.to_hex());
}
