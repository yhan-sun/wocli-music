use rand::seq::SliceRandom;
#[derive(Default)]
pub struct Shuffle {
    remaining: Vec<usize>,
    history: Vec<usize>,
    cursor: usize,
}
impl Shuffle {
    pub fn reset(&mut self, size: usize, current: usize) {
        self.remaining = (0..size).filter(|&v| v != current).collect();
        self.remaining.shuffle(&mut rand::thread_rng());
        self.history = if current < size {
            vec![current]
        } else {
            vec![]
        };
        self.cursor = 0;
    }
    pub fn next(&mut self, size: usize, current: usize) -> usize {
        if self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            return self.history[self.cursor];
        }
        if self.remaining.is_empty() {
            self.remaining = (0..size).collect();
            self.remaining.shuffle(&mut rand::thread_rng());
            if size > 1 && self.remaining.last() == Some(&current) {
                self.remaining.swap(0, size - 1);
            }
        }
        let next = self.remaining.pop().unwrap_or(current);
        self.history.push(next);
        self.cursor = self.history.len() - 1;
        next
    }
    pub fn previous(&mut self) -> Option<usize> {
        if self.cursor == 0 {
            None
        } else {
            self.cursor -= 1;
            Some(self.history[self.cursor])
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shuffle_round_and_history() {
        let mut order = Shuffle::default();
        order.reset(4, 0);
        let a = order.next(4, 0);
        let b = order.next(4, a);
        let c = order.next(4, b);
        let mut played = vec![0, a, b, c];
        played.sort();
        assert_eq!(played, vec![0, 1, 2, 3]);
        assert_eq!(order.previous(), Some(b));
        assert_eq!(order.next(4, b), c);
        assert_ne!(order.next(4, c), c);
    }
}
