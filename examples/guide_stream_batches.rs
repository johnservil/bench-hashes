// Batches of equal-length messages, one after another, each written straight
// into the stream: its threads hash them side by side while you write the
// next batch, each digest into the batch's place, which comes back whole.
use blake3_servil::{Hash, Mode};
use std::cell::RefCell;

const MESSAGES: usize = 1024; // each batch: 1024 messages of 64 bytes

fn main() {
    // Once per process, before any other call.
    blake3_servil::initialize_multithreaded();
    // A fixed set of batch places, cycled.
    let mut places = vec![Hash::from_bytes([0; 32]); 8 * MESSAGES];
    let free = RefCell::new(places.chunks_mut(MESSAGES).collect::<Vec<_>>());
    blake3_servil::stream(
        Mode::Hash,
        |done: &mut [Hash]| {
            println!("{}", done[0].to_hex()); // the batch's first digest
            free.borrow_mut().push(done);
        },
        |s| {
            for batch_number in 0..100u8 {
                let place = loop {
                    if let Some(place) = free.borrow_mut().pop() {
                        break place;
                    }
                    s.collect();
                };
                // Write the batch's bytes from `at` into `room`: each 64-byte
                // message in its 64-byte slot (shorter ones zero-padded).
                s.submit_batch_with(64, |_at, room| room.fill(batch_number), place); // your read goes here
            }
            s.finish();
        },
    );
}
