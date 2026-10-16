// Messages one after another, each read straight into the stream. Its
// threads hash earlier messages while you read the next; each digest goes
// into a place you own, and the place comes back to your handler.
use blake3_servil::{Hash, Mode};
use std::cell::RefCell;

fn main() {
    // Once per process, before any other call.
    blake3_servil::initialize_multithreaded();
    // A fixed set of places, cycled: each comes back with its digest in it.
    let mut places = vec![Hash::from_bytes([0; 32]); 64];
    let free = RefCell::new(places.iter_mut().collect::<Vec<_>>());
    blake3_servil::stream(
        Mode::Hash,
        // Your handler receives each place back, in submission order, on
        // this thread. Keep it short.
        |done: &mut [Hash]| {
            println!("{}", done[0].to_hex());
            free.borrow_mut().push(&mut done[0]);
        },
        |s| {
            for message_number in 0..1000u32 {
                // Take a free place, or hand finished ones back until one is.
                let place = loop {
                    if let Some(place) = free.borrow_mut().pop() {
                        break place;
                    }
                    s.collect();
                };
                let message = message_number.to_le_bytes(); // your read goes here
                s.submit_with(message.len(), |at, room| room.copy_from_slice(&message[at..][..room.len()]), place);
            }
            // Every place comes back before the stream ends.
            s.finish();
        },
    );
}
