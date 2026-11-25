//! Digests, for the protocols that need them: MD5 and SHA-256, the keyed digest and the
//! key derivation built on SHA-256 (HMAC, PBKDF2), and base64.
//!
//! A database asks for these to check a password without the password being sent
//! (`pg.rs`). The constants below are what the two digests define: for SHA-256 the
//! fractional parts of the square and cube roots of the first primes, and for MD5 of the
//! sines of the whole numbers.

const SHA_START: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
const SHA_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];
const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8,
    0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340,
    0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87,
    0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039,
    0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92,
    0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
    0xeb86d391,
];

/// Whether the processor has instructions for SHA-256, which are ten times as fast as
/// doing it by hand. Every Apple processor of this kind has them; on Linux the system
/// says.
#[cfg(target_arch = "aarch64")]
fn processor_does_sha() -> bool {
    use core::sync::atomic::{AtomicU8, Ordering::Relaxed};
    static KNOWN: AtomicU8 = AtomicU8::new(0);
    match KNOWN.load(Relaxed) {
        0 => {
            let has = if cfg!(target_os = "macos") {
                true
            } else {
                unsafe extern "C" {
                    fn getauxval(kind: u64) -> u64;
                }
                // The capabilities of the hardware, of which the sixth bit is SHA-2.
                unsafe { getauxval(16) & (1 << 6) != 0 }
            };
            KNOWN.store(if has { 2 } else { 1 }, Relaxed);
            has
        }
        known => known == 2,
    }
}

/// One block, by the processor's own instructions: sixteen steps of four rounds, each
/// of which also makes the four words of the message that are needed four steps on.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "sha2")]
unsafe fn compress_by_processor(state: &mut [u32; 8], block: &[u8; 64]) {
    use core::arch::aarch64::*;
    unsafe {
        let (first, second) = (vld1q_u32(state.as_ptr()), vld1q_u32(state.as_ptr().add(4)));
        let (mut low, mut high) = (first, second);
        let mut words = [
            vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block.as_ptr()))),
            vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block.as_ptr().add(16)))),
            vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block.as_ptr().add(32)))),
            vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block.as_ptr().add(48)))),
        ];
        for step in 0..16 {
            let at = step % 4;
            let keyed = vaddq_u32(words[at], vld1q_u32(SHA_K.as_ptr().add(4 * step)));
            let before = low;
            low = vsha256hq_u32(low, high, keyed);
            high = vsha256h2q_u32(high, before, keyed);
            if step < 12 {
                words[at] = vsha256su1q_u32(
                    vsha256su0q_u32(words[at], words[(at + 1) % 4]),
                    words[(at + 2) % 4],
                    words[(at + 3) % 4],
                );
            }
        }
        vst1q_u32(state.as_mut_ptr(), vaddq_u32(low, first));
        vst1q_u32(state.as_mut_ptr().add(4), vaddq_u32(high, second));
    }
}

#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    total: u64,
}

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256 {
            state: SHA_START,
            block: [0; 64],
            filled: 0,
            total: 0,
        }
    }

    fn compress(&mut self) {
        #[cfg(test)]
        if tests::BY_HAND.load(core::sync::atomic::Ordering::Relaxed) {
            return self.compress_by_hand();
        }
        #[cfg(target_arch = "aarch64")]
        if processor_does_sha() {
            unsafe { compress_by_processor(&mut self.state, &self.block) };
            return;
        }
        self.compress_by_hand();
    }

    fn compress_by_hand(&mut self) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                self.block[4 * i],
                self.block[4 * i + 1],
                self.block[4 * i + 2],
                self.block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(SHA_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (state, add) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *state = state.wrapping_add(add);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total += data.len() as u64;
        while !data.is_empty() {
            let take = data.len().min(64 - self.filled);
            self.block[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == 64 {
                self.compress();
                self.filled = 0;
            }
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.total * 8;
        self.update(&[0x80]);
        if self.filled > 56 {
            self.block[self.filled..].fill(0);
            self.compress();
            self.filled = 0;
        }
        self.block[self.filled..56].fill(0);
        self.block[56..].copy_from_slice(&bits.to_be_bytes());
        self.compress();
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(data);
    digest.finish()
}

/// A key, ready to make keyed digests: the two digests that every one of them begins
/// with, so that the key is gone over once however many messages there are.
#[derive(Clone)]
pub struct Hmac {
    inner: Sha256,
    outer: Sha256,
}

impl Hmac {
    pub fn new(key: &[u8]) -> Hmac {
        let mut padded = [0u8; 64];
        if key.len() > 64 {
            padded[..32].copy_from_slice(&sha256(key));
        } else {
            padded[..key.len()].copy_from_slice(key);
        }
        let (mut inner, mut outer) = (Sha256::new(), Sha256::new());
        inner.update(&padded.map(|byte| byte ^ 0x36));
        outer.update(&padded.map(|byte| byte ^ 0x5c));
        Hmac { inner, outer }
    }

    /// The keyed digest of the parts of a message, one after the other.
    pub fn of(&self, parts: &[&[u8]]) -> [u8; 32] {
        let mut inner = self.inner.clone();
        for part in parts {
            inner.update(part);
        }
        let mut outer = self.outer.clone();
        outer.update(&inner.finish());
        outer.finish()
    }
}

/// The keyed digest of the parts of a message, one after the other.
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    Hmac::new(key).of(parts)
}

/// A key derived from a password and a salt by so many rounds, for one block of output.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let keyed = Hmac::new(password);
    let mut round = keyed.of(&[salt, &[0, 0, 0, 1]]);
    let mut out = round;
    for _ in 1..rounds {
        round = keyed.of(&[&round]);
        for (byte, add) in out.iter_mut().zip(round) {
            *byte ^= add;
        }
    }
    out
}

/// The MD5 digest of the parts of a message, one after the other.
pub fn md5(parts: &[&[u8]]) -> [u8; 16] {
    const SHIFTS: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut block = [0u8; 64];
    let mut filled = 0;
    let mut total: u64 = 0;
    let compress = |block: &[u8; 64], state: &mut [u32; 4]| {
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            *word = u32::from_le_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        let [mut a, mut b, mut c, mut d] = *state;
        for i in 0..64 {
            let (mix, word) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let turned = a
                .wrapping_add(mix)
                .wrapping_add(MD5_K[i])
                .wrapping_add(m[word])
                .rotate_left(SHIFTS[(i / 16) * 4 + i % 4]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(turned);
        }
        for (state, add) in state.iter_mut().zip([a, b, c, d]) {
            *state = state.wrapping_add(add);
        }
    };
    let feed = |data: &[u8], block: &mut [u8; 64], filled: &mut usize, state: &mut [u32; 4]| {
        for byte in data {
            block[*filled] = *byte;
            *filled += 1;
            if *filled == 64 {
                compress(block, state);
                *filled = 0;
            }
        }
    };
    for part in parts {
        total += part.len() as u64;
        feed(part, &mut block, &mut filled, &mut state);
    }
    feed(&[0x80], &mut block, &mut filled, &mut state);
    while filled != 56 {
        feed(&[0], &mut block, &mut filled, &mut state);
    }
    feed(&(total * 8).to_le_bytes(), &mut block, &mut filled, &mut state);
    let mut out = [0u8; 16];
    for (i, word) in state.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Writes bytes as base64 into `out`, and returns how many characters that is.
pub fn base64_encode(data: &[u8], out: &mut [u8]) -> usize {
    let mut at = 0;
    for chunk in data.chunks(3) {
        let bits =
            (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out[at + i] = if i <= chunk.len() {
                BASE64[(bits >> shift) as usize & 63]
            } else {
                b'='
            };
        }
        at += 4;
    }
    at
}

/// Reads base64 into `out`, and returns how many bytes it was, or nothing if it is not
/// base64 or does not fit.
pub fn base64_decode(text: &[u8], out: &mut [u8]) -> Option<usize> {
    let text = match text {
        [rest @ .., b'=', b'='] | [rest @ .., b'='] => rest,
        all => all,
    };
    let (mut bits, mut held, mut at) = (0u32, 0, 0);
    for byte in text {
        let value = BASE64.iter().position(|letter| letter == byte)? as u32;
        bits = bits << 6 | value;
        held += 6;
        if held >= 8 {
            held -= 8;
            *out.get_mut(at)? = (bits >> held) as u8;
            at += 1;
        }
    }
    Some(at)
}

/// Writes bytes as lower-case hexadecimal into `out`, which is twice as long.
pub fn hex(data: &[u8], out: &mut [u8]) {
    for (i, byte) in data.iter().enumerate() {
        out[2 * i] = b"0123456789abcdef"[(byte >> 4) as usize];
        out[2 * i + 1] = b"0123456789abcdef"[(byte & 15) as usize];
    }
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicBool, Ordering::Relaxed};

    /// Set to make the digest by hand even where the processor could.
    pub static BY_HAND: AtomicBool = AtomicBool::new(false);

    #[test]
    fn by_hand_is_as_by_the_processor() {
        let mut message = [0u8; 300];
        for (index, byte) in message.iter_mut().enumerate() {
            *byte = (index * 7 + 3) as u8;
        }
        for length in [0, 1, 55, 56, 57, 63, 64, 65, 119, 120, 128, 300] {
            let either = super::sha256(&message[..length]);
            BY_HAND.store(true, Relaxed);
            let by_hand = super::sha256(&message[..length]);
            BY_HAND.store(false, Relaxed);
            assert_eq!(either, by_hand, "a message of {length} bytes");
        }
        BY_HAND.store(true, Relaxed);
        let abc = super::sha256(b"abc");
        BY_HAND.store(false, Relaxed);
        assert_eq!(abc[..4], [0xba, 0x78, 0x16, 0xbf]);
    }

    use super::*;

    fn shown(bytes: &[u8]) -> String {
        let mut out = vec![0u8; bytes.len() * 2];
        hex(bytes, &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn digests_are_those_of_the_standards() {
        assert_eq!(shown(&md5(&[b""])), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(
            shown(&md5(&[b"The quick brown ", b"fox jumps over the lazy dog"])),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        assert_eq!(shown(&md5(&[&[b'a'; 200]])), "887f30b43b2867f4a9accceee7d16e6c");
        assert_eq!(
            shown(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            shown(&sha256(&[b'a'; 200])),
            "c2a908d98f5df987ade41b5fce213067efbcc21ef2240212a41e54b5e7c28ae5"
        );
        assert_eq!(
            shown(&hmac_sha256(
                b"key",
                &[b"The quick brown fox ", b"jumps over the lazy dog"]
            )),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
        assert_eq!(
            shown(&pbkdf2_sha256(b"pencil", b"salt-salt", 4096)),
            "3457cf6c74184c9d50ee3681bcb53ebe99f75c942dd9448ae54cf9068233d99e"
        );
    }

    #[test]
    fn base64_goes_there_and_back() {
        for text in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            let mut encoded = [0u8; 16];
            let length = base64_encode(text, &mut encoded);
            let mut decoded = [0u8; 16];
            let back = base64_decode(&encoded[..length], &mut decoded).unwrap();
            assert_eq!(&decoded[..back], text);
        }
        let mut encoded = [0u8; 16];
        let length = base64_encode(b"foobar", &mut encoded);
        assert_eq!(&encoded[..length], b"Zm9vYmFy");
        let length = base64_encode(b"fo", &mut encoded);
        assert_eq!(&encoded[..length], b"Zm8=");
        assert!(base64_decode(b"not base64!", &mut [0u8; 16]).is_none());
    }
}
