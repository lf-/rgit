//! Myers diff algorithm
use std::convert::TryFrom;
use std::fmt;

/// Allows negative indexing into slices in a similar fashion to Python and Ruby
trait NegIndex<T> {
    fn nindex(&self, idx: isize) -> &T;
    fn nindex_mut(&mut self, idx: isize) -> &mut T;
}

impl<T> NegIndex<T> for [T] {
    fn nindex(&self, idx: isize) -> &T {
        // normal forward indexing
        if idx >= 0 {
            &self[idx as usize]
        } else {
            &self[self.len() - ((-idx) as usize)]
        }
    }
    fn nindex_mut(&mut self, idx: isize) -> &mut T {
        // normal forward indexing
        if idx >= 0 {
            &mut self[idx as usize]
        } else {
            &mut self[self.len() - ((-idx) as usize)]
        }
    }
}

/// Change in a list
#[derive(Debug, Eq, PartialEq)]
pub enum Edit<'a, T> {
    /// Element was added
    Ins(&'a T),
    /// Element was deleted
    Del(&'a T),
    /// Element is unchanged
    Nop(&'a T),
}

/// Perform a diff on two slices of arbitrary objects using the Myers algorithm.
/// Returns a list of edits to `a` that would produce `b`.
pub fn myers_diff<'a, T>(a: &'a [T], b: &'a [T]) -> Vec<Edit<'a, T>>
where
    T: Eq + fmt::Debug,
{
    let moves = myers_backtrack(a, b);

    // The moves are stored backwards so we iterate backwards
    moves
        .into_iter()
        .rev()
        .map(|((old_del, old_ins), (new_del, new_ins))| {
            if old_del == new_del {
                // An insert happened if there were no deletions
                Edit::Ins(&b[old_ins])
            } else if old_ins == new_ins {
                // A deletion happened if there were no insertions
                Edit::Del(&a[old_del])
            } else {
                // There were insertions and deletions so it was a diagonal
                // move -> equal
                assert_eq!(&a[old_del], &b[old_ins]);
                Edit::Nop(&a[old_del])
            }
        })
        .collect()
}

/// Perform a backtracking Myers diff between two lists of comparable items and
/// return a reversed list of `((old del, old ins), (new del, new ins))` to reach
/// list `b`.
fn myers_backtrack<T>(a: &[T], b: &[T]) -> Vec<((usize, usize), (usize, usize))>
where
    T: Eq,
{
    let mut diffs = Vec::new();
    let mut x = a.len();
    let mut y = b.len();

    let trace = myers_trace(a, b);
    for (d, v) in trace.iter().enumerate().rev() {
        let k = isize::try_from(x).unwrap() - isize::try_from(y).unwrap();
        let d = isize::try_from(d).unwrap();

        println!("k: {}\td: {}\tv: {:?}", k, d, &v);

        // find what the previous k would have been using the same logic as the
        // forward direction
        let k_was = if k == -d || (k != d && v.nindex(k - 1).unwrap() < v.nindex(k + 1).unwrap()) {
            k + 1
        } else {
            k - 1
        };

        // Previous x and y may be negative at d = 0 (first edit step)
        let x_was = isize::try_from(v.nindex(k_was).unwrap()).unwrap();
        let y_was = x_was - k_was;
        println!("({}, {}) -> ({}, {})", x_was, y_was, x, y);

        while isize::try_from(x).unwrap() > x_was && isize::try_from(y).unwrap() > y_was {
            // diagonal move
            diffs.push(((x - 1, y - 1), (x, y)));
            x -= 1;
            y -= 1;
        }

        // For all except the first change, record the previous x and y.
        // The x_was and y_was for the first change at d = 0 may be negative
        // (invalid).
        if d > 0 {
            // These should never be negative. Assert that is in fact the case.
            let x_was = usize::try_from(x_was).unwrap();
            let y_was = usize::try_from(y_was).unwrap();
            diffs.push(((x_was, y_was), (x, y)));
            x = x_was;
            y = y_was;
        }
    }
    diffs
}

/// Finds the most efficient edit sequence and outputs a list of state arrays to
/// reach it.
fn myers_trace<T>(a: &[T], b: &[T]) -> Vec<Vec<Option<usize>>>
where
    T: Eq,
{
    let mut traces = Vec::new();

    let n = a.len();
    let m = b.len();
    let max = n + m;
    // state array
    let mut v = Vec::with_capacity(2 * max + 1);

    // x is deletions, y is insertions. This algorithm is designed to maximize
    // deletions while finding diffs.

    // At each position, the "best" possible previous position is selected. This
    // is chosen by finding the one with the largest x value since we maximize
    // deletions.
    //
    // d is the depth in the graph, k is (x - y). On each new node, one of
    // three changes can happen to k when looking at depth d - 1:
    // * rightward move (deletion): k decremented
    // * downward move (insertion): k incremented
    // * diagonal move (same): k unchanged

    // The state array has even and odd values of k modified on alternating
    // iterations. It stores the newest values of x for each value of k. The
    // algorithm selects the largest value of x (deletions) for each iteration.

    // Fill state array with placeholders
    for _ in 0..(2 * max + 1) {
        v.push(None);
    }

    // Initial depth should select x = 0
    v[1] = Some(0usize);

    // Iterate through d depths
    for d in 0..=max as isize {
        let mut x;
        let mut y;
        traces.push(v.clone());

        for k in (-d..=d).step_by(2) {
            if k == -d || (k != d && v.nindex(k - 1).unwrap() < v.nindex(k + 1).unwrap()) {
                // Move downwards
                x = v.nindex(k + 1).unwrap();
            } else {
                // Move right: x will be one greater than the previous round
                x = v.nindex(k - 1).unwrap() + 1;
            }
            let ytemp = x as isize - k;
            assert!(ytemp >= 0);
            y = ytemp as usize;

            // Try to take diagonal steps
            while x < n && y < m && a[x] == b[y] {
                x += 1;
                y += 1;
            }
            //println!("({}, {})\tk: {}\td: {}", x, y, k, d);

            *v.nindex_mut(k) = Some(x);
            if x >= n && y >= m {
                // Reached the bottom right position. Report it
                return traces;
            }
        }
    }
    unreachable!("failed to diff??")
}

#[cfg(test)]
mod test {
    use super::Edit;
    use super::NegIndex;

    #[test]
    fn test_myers() {
        // stolen from the Ruby implementation. if we can't understand it, at
        // least we can ensure we're doing the same thing.
        #[rustfmt::skip]
        let good_trace = vec![
            vec![
                None, Some(0), None, None, None, None, None, None, None, None, None, None, None,
            ],
            vec![
                Some(1), Some(0), None, None, None, None, None, None, None, None, None, None, None,
            ],
            vec![
                Some(1), Some(3), None, None, None, None, None, None, None, None, None, None, Some(2),
            ],
        ];

        assert_eq!(super::myers_trace(b"abc", b"acb"), good_trace);

        assert_eq!(
            super::myers_backtrack(b"abc", b"acb"),
            vec![
                ((3, 2), (3, 3)), // insert b
                ((2, 1), (3, 2)), // diagonal (c is same)
                ((1, 1), (2, 1)), // delete b
                ((0, 0), (1, 1))  // diagonal move (insert+delete) => "a" is same
            ]
        );

        assert_eq!(
            super::myers_backtrack(b"abc", b"abc"),
            // three diagonal moves (same character)
            vec![((2, 2), (3, 3)), ((1, 1), (2, 2)), ((0, 0), (1, 1))]
        );

        assert_eq!(
            super::myers_diff(b"abc", b"acb"),
            vec![
                Edit::Nop(&b'a'),
                Edit::Del(&b'b'),
                Edit::Nop(&b'c'),
                Edit::Ins(&b'b')
            ]
        );
    }

    #[test]
    fn test_nindex() {
        let v = vec![1, 2, 3, 4];
        assert_eq!(*v.nindex(-1), 4);
        assert_eq!(*v.nindex(-2), 3);
        assert_eq!(*v.nindex(0), 1);
    }
}
