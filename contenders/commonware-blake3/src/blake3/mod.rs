//! The parts of commonware-cryptography's `blake3` module (commit 3aa183f0)
//! that its batch kernels use, and its `Blake3::hash_many`.

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
#[allow(dead_code)] // the pair and parts kernels, which no use case calls
mod simd;

const DIGEST_LENGTH: usize = blake3::OUT_LEN;

const PAIR_LEN: usize = 2 * blake3::BLOCK_LEN;

/// Copy the concatenation of `parts` into a zero-padded buffer, returning
/// `None` if it exceeds [`PAIR_LEN`] bytes.
#[inline]
#[allow(dead_code)]
fn gather(parts: &[&[u8]]) -> Option<([u8; PAIR_LEN], usize)> {
    let mut buffer = [0u8; PAIR_LEN];
    let mut len = 0;
    for part in parts {
        buffer.get_mut(len..len + part.len())?.copy_from_slice(part);
        len += part.len();
    }
    Some((buffer, len))
}

/// Digest of a BLAKE3 hashing operation.
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
#[repr(transparent)]
pub struct Digest(pub [u8; DIGEST_LENGTH]);

impl From<blake3::Hash> for Digest {
    fn from(value: blake3::Hash) -> Self {
        Self(value.into())
    }
}

/// `Blake3::hash_many`: one digest per message, in order.
pub fn hash_many<M: AsRef<[u8]>>(messages: &[M]) -> Vec<Digest> {
    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    if let Some(digests) = simd::hash_many(messages) {
        return digests;
    }
    messages
        .iter()
        .map(|message| blake3::hash(message.as_ref()).into())
        .collect()
}
