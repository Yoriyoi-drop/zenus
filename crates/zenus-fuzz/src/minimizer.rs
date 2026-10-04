use alloc::vec::Vec;

/// Minimize a crashing input while preserving the crash
pub fn minimize(input: &[u8]) -> Vec<u8> {
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

/// Check if the input still causes a crash
fn preserves_crash(input: &[u8]) -> bool {
    // In a real implementation, this would execute the input and check for crash
    // For now, we use a simple heuristic
    !input.is_empty()
}

/// Minimize multiple inputs
pub fn minimize_all(inputs: &[Vec<u8>]) -> Vec<Vec<u8>> {
    inputs.iter().map(|input| minimize(input)).collect()
}
