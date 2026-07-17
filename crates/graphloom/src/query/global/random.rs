//! Minimal CPython-compatible MT19937 shuffle used by GraphRAG seed 86.

const STATE_SIZE: usize = 624;
const PERIOD: usize = 397;

#[derive(Debug, Clone)]
pub(super) struct PythonRandom {
    state: [u32; STATE_SIZE],
    index: usize,
}

impl PythonRandom {
    pub(super) fn new(seed: u32) -> Self {
        let mut value = Self {
            state: [0; STATE_SIZE],
            index: STATE_SIZE,
        };
        value.init_by_array(&[seed]);
        value
    }

    pub(super) fn shuffle<T>(&mut self, values: &mut [T]) {
        for upper in (1..values.len()).rev() {
            let selected = self.rand_below(upper + 1);
            values.swap(upper, selected);
        }
    }

    fn init_genrand(&mut self, seed: u32) {
        self.state[0] = seed;
        for index in 1..STATE_SIZE {
            self.state[index] = 1_812_433_253_u32
                .wrapping_mul(self.state[index - 1] ^ (self.state[index - 1] >> 30))
                .wrapping_add(index as u32);
        }
        self.index = STATE_SIZE;
    }

    fn init_by_array(&mut self, keys: &[u32]) {
        self.init_genrand(19_650_218);
        let mut i = 1_usize;
        let mut j = 0_usize;
        for _ in 0..STATE_SIZE.max(keys.len()) {
            self.state[i] = (self.state[i]
                ^ (self.state[i - 1] ^ (self.state[i - 1] >> 30)).wrapping_mul(1_664_525))
            .wrapping_add(keys[j])
            .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= STATE_SIZE {
                self.state[0] = self.state[STATE_SIZE - 1];
                i = 1;
            }
            if j >= keys.len() {
                j = 0;
            }
        }
        for _ in 0..(STATE_SIZE - 1) {
            self.state[i] = (self.state[i]
                ^ (self.state[i - 1] ^ (self.state[i - 1] >> 30)).wrapping_mul(1_566_083_941))
            .wrapping_sub(i as u32);
            i += 1;
            if i >= STATE_SIZE {
                self.state[0] = self.state[STATE_SIZE - 1];
                i = 1;
            }
        }
        self.state[0] = 0x8000_0000;
    }

    fn rand_below(&mut self, upper: usize) -> usize {
        let bits = usize::BITS - upper.leading_zeros();
        loop {
            let candidate = self.getrandbits(bits) as usize;
            if candidate < upper {
                return candidate;
            }
        }
    }

    fn getrandbits(&mut self, bits: u32) -> u32 {
        debug_assert!((1..=32).contains(&bits));
        self.genrand_uint32() >> (32 - bits)
    }

    fn genrand_uint32(&mut self) -> u32 {
        if self.index >= STATE_SIZE {
            for index in 0..STATE_SIZE {
                let value = (self.state[index] & 0x8000_0000)
                    | (self.state[(index + 1) % STATE_SIZE] & 0x7fff_ffff);
                self.state[index] = self.state[(index + PERIOD) % STATE_SIZE]
                    ^ (value >> 1)
                    ^ if value & 1 == 0 { 0 } else { 0x9908_b0df };
            }
            self.index = 0;
        }
        let mut value = self.state[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^ (value >> 18)
    }
}

#[cfg(test)]
mod tests {
    use super::PythonRandom;

    #[test]
    fn test_should_match_python_seed_86_shuffle_permutations() {
        let expected = [
            vec![0],
            vec![1, 0],
            vec![2, 1, 0],
            vec![3, 1, 2, 0],
            vec![1, 4, 3, 2, 0],
            vec![1, 5, 3, 2, 4, 0],
            vec![1, 5, 3, 2, 4, 0, 6],
            vec![3, 1, 5, 6, 7, 2, 4, 0],
        ];
        for (length, expected) in (1_usize..).zip(expected) {
            let mut values = (0..length).collect::<Vec<_>>();
            PythonRandom::new(86).shuffle(&mut values);
            assert_eq!(values, expected);
        }
    }
}
