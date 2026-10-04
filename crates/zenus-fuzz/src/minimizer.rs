use alloc::vec::Vec;

/// Minimize a crashing input while preserving the crash
pub fn minimize(input: &[u8]) -> Vec<u8> {
    // The default predicate cannot actually run the input (that needs the
    // fault-containment machinery), so it only refuses to shrink to nothing.
    minimize_with(input, |candidate| !candidate.is_empty())
}

/// Minimize with an injected "does this still crash?" predicate.
///
/// `minimize` used to hard-code its predicate, which made the whole delta
/// debugging loop untestable — and hid the fact that the built-in predicate
/// does not reproduce anything. With the predicate injected, the search itself
/// (chunk removal, byte zeroing, fixed point) is testable against a real
/// oracle.
pub fn minimize_with(input: &[u8], mut preserves_crash: impl FnMut(&[u8]) -> bool) -> Vec<u8> {
    let mut current = input.to_vec();
    let mut changed = true;

    while changed {
        changed = false;

        // Try removing chunks
        let mut chunk_size = current.len() / 2;
        while chunk_size > 0 {
            let mut i = 0;
            while i < current.len() {
                let end = (i + chunk_size).min(current.len());
                let mut candidate = Vec::with_capacity(current.len() - (end - i));
                candidate.extend_from_slice(&current[..i]);
                candidate.extend_from_slice(&current[end..]);

                if candidate.len() < current.len() && preserves_crash(&candidate) {
                    current = candidate;
                    changed = true;
                } else {
                    i += chunk_size;
                }
            }
            chunk_size /= 2;
        }

        // Try replacing bytes with zeros
        for i in 0..current.len() {
            let original = current[i];
            if original == 0 {
                // Already zero: "zeroing" it changes nothing, so it must not
                // count as progress. It used to, and the outer `while changed`
                // never terminated — a fuzz campaign would hang the moment a
                // minimised input contained a zero byte.
                continue;
            }
            current[i] = 0;
            if !preserves_crash(&current) {
                current[i] = original;
            } else {
                changed = true;
            }
        }
    }

    current
}

/// Minimize multiple inputs
pub fn minimize_all(inputs: &[Vec<u8>]) -> Vec<Vec<u8>> {
    inputs.iter().map(|input| minimize(input)).collect()
}
