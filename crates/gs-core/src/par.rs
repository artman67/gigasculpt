//! Node-list helpers.

/// Mutable references to `items[i]` for each `i` in `ids`, which must be sorted and distinct.
pub(crate) fn select_mut<'a, T>(items: &'a mut [T], ids: &[u32]) -> Vec<&'a mut T> {
    let mut out = Vec::with_capacity(ids.len());
    let mut rest = items;
    let mut at = 0;
    for &id in ids {
        let (_, tail) = rest.split_at_mut(id as usize - at);
        let Some((x, tail)) = tail.split_first_mut() else {
            panic!("node id {id} out of range");
        };
        out.push(x);
        rest = tail;
        at = id as usize + 1;
    }
    out
}

/// Sorted union of two sorted id lists.
pub(crate) fn merge(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut v = [a, b].concat();
    v.sort_unstable();
    v.dedup();
    v
}

/// [`merge`] in one pass, for long lists: both must be ascending.
pub(crate) fn union(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::with_capacity(a.len() + b.len());
    let (mut a, mut b) = (a.iter().peekable(), b.iter().peekable());
    loop {
        let x = match (a.peek(), b.peek()) {
            (Some(&&x), Some(&&y)) if x <= y => a.next().copied(),
            (_, Some(_)) => b.next().copied(),
            (Some(_), None) => a.next().copied(),
            (None, None) => break,
        };
        if let Some(x) = x.filter(|x| out.last() != Some(x)) {
            out.push(x);
        }
    }
    out
}
