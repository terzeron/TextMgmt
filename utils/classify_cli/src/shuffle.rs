//! Python random.Random(integer).shuffle와 같은 MT19937 순서. 표본을 바꾸지 않기 위해 사용한다.
pub(super) struct Random {
    mt: [u32; 624],
    index: usize,
}
impl Random {
    pub(super) fn new(seed: u64) -> Self {
        let mut r = Self { mt: [0; 624], index: 624 };
        r.mt[0] = 19650218;
        for i in 1..624 {
            r.mt[i] = 1812433253u32.wrapping_mul(r.mt[i - 1] ^ (r.mt[i - 1] >> 30)).wrapping_add(i as u32);
        }
        let mut key = vec![seed as u32];
        if seed >> 32 != 0 {
            key.push((seed >> 32) as u32);
        }
        let (mut i, mut j) = (1, 0);
        for _ in 0..624 {
            r.mt[i] = (r.mt[i] ^ (r.mt[i - 1] ^ (r.mt[i - 1] >> 30)).wrapping_mul(1664525)).wrapping_add(key[j]).wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                r.mt[0] = r.mt[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            r.mt[i] = (r.mt[i] ^ (r.mt[i - 1] ^ (r.mt[i - 1] >> 30)).wrapping_mul(1566083941)).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                r.mt[0] = r.mt[623];
                i = 1;
            }
        }
        r.mt[0] = 0x80000000;
        r
    }
    fn next(&mut self) -> u32 {
        if self.index == 624 {
            for i in 0..624 {
                let y = (self.mt[i] & 0x80000000) | (self.mt[(i + 1) % 624] & 0x7fffffff);
                self.mt[i] = self.mt[(i + 397) % 624] ^ (y >> 1) ^ if y & 1 != 0 { 0x9908b0df } else { 0 };
            }
            self.index = 0;
        }
        let mut y = self.mt[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680;
        y ^= (y << 15) & 0xefc60000;
        y ^= y >> 18;
        y
    }
    pub(super) fn below(&mut self, n: usize) -> usize {
        let bits = usize::BITS - n.leading_zeros();
        loop {
            let value = if bits <= 32 { (self.next() >> (32 - bits)) as u64 } else { u64::from(self.next()) | (u64::from(self.next() >> (64 - bits)) << 32) };
            if value < n as u64 {
                return value as usize;
            }
        }
    }
}
pub(super) fn shuffle<T>(values: &mut [T], seed: u64) {
    let mut random = Random::new(seed);
    for i in (1..values.len()).rev() {
        let j = random.below(i + 1);
        values.swap(i, j);
    }
}
